//! キャンバスの表示の合成を GPU（yolu-gpu の常駐の合成）にした道の試験: CPU の表示と同じ絵になること（レイヤー・マスク・合成モード・
//! クリッピング・グループ）、描いたタイルだけを更新すること、使えない文書・予算・上限を超えるときに理由を覚えて CPU へ落ちて
//! 毎フレームは試し直さないこと、戻ること。ここでは方針を `Gpu` にして GPU の道を強制する（`Auto` はソフトウェアのアダプター
//! では CPU）。コンテナの実 GPU や Windows の結果ではない。
//!
//! 描画器は egui_kittest の wgpu で、`canvas_device` が Vulkan・Metal・DX12・GL の順に、製品の egui デバイスと同じ上限で動く
//! アダプターだけを選ぶ（コンテナでは Vulkan の lavapipe。GL の egui デバイスは WebGL2 相当の上限で compute/storage が足りず
//! 選ばれない）。環境変数 `WGPU_BACKEND` があればその範囲だけを調べる。使える描画器が 1 つもなければ、各試験は理由と未検証
//! 事項を標準エラーへ「省略」として出して成功のまま抜ける（全部の試験が省略になる。GPU 合成の保証はその環境では確かめて
//! いない）。選んだデバイスは試験どうしで共有するため、ウィンドウを捨てるまで試験は直列になる。
use crate::common;
use crate::common::canvas_device;

use common::*;
use egui::{vec2, Pos2, Rect};
use egui_kittest::Harness;
use yolu_app::canvas::gpu::{CanvasBackend, FailureKind, Fallback, Shown};
use yolu_app::engine::{
    AdjustmentSettings, BlendMode, Channel, ChannelBlend, Document, LayerId, Rect as DocRect, Rgba8,
};
use yolu_app::lang::Lang;
use yolu_app::pen::PenInput;
use yolu_app::state::{Action, AppState};
use yolu_app::YoluApp;
use yolu_gpu::Unsupported;

/// 1280 × 800 のウィンドウに、doc_w × doc_h の文書。
fn canvas_app(doc_w: u32, doc_h: u32, policy: CanvasBackend) -> Harness<'static, YoluApp> {
    let mut h = common::gpu_thread::builder()
        .with_size(vec2(1280.0, 800.0))
        .with_pixels_per_point(1.0)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .with_render_options(render_options())
        .renderer(canvas_device::renderer())
        .build_eframe(move |cc| {
            let mut app = YoluApp::for_context(
                &cc.egui_ctx,
                AppState::new(doc_w, doc_h),
                PenInput::detached(),
            )
            .with_render_state(cc.wgpu_render_state.as_ref());
            app.set_canvas_backend(policy);
            // 中央は 1 つの組（キャンバスだけが広く出る）
            app.dock = common::tabbed_center_dock(1280.0);
            app
        });
    h.run();
    h
}

struct Rng(u32);
impl Rng {
    fn byte(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0 as u8
    }
}

fn paint(d: &mut Document, layer: LayerId, rng: &mut Rng, alphas: &[u8]) {
    for y in 0..d.height() {
        for x in 0..d.width() {
            let a = alphas[(x as usize / 5 + y as usize / 7) % alphas.len()];
            d.set_pixel(
                layer,
                x,
                y,
                Rgba8::new(rng.byte(), rng.byte(), rng.byte(), a),
            )
            .unwrap();
        }
    }
}

/// アプリの文書の機能を重ねた文書（下地・マスク付きの乗算・クリッピングのスクリーン・塗りつぶしのオーバーレイ・
/// グループの中のクリッピング・チャンネルごとの合成）。どのレイヤーも core の編集の口で作る。
fn rich_document(d: &mut Document) {
    let mut rng = Rng(0x9e3779b9);
    let base = d.layers()[0].id();
    paint(d, base, &mut rng, &[255]);
    let a = d.add_layer("乗算").unwrap();
    paint(d, a, &mut rng, &[255, 200, 90, 255]);
    d.set_layer_blend_mode(a, BlendMode::Multiply).unwrap();
    d.set_layer_opacity(a, 0.8, false).unwrap();
    d.add_layer_mask(a).unwrap();
    for y in 0..d.height() {
        for x in 0..d.width() {
            let hide = if (x / 16 + y / 16) % 2 == 0 { 0 } else { 140 };
            d.set_mask_pixel(a, x, y, hide).unwrap();
        }
    }
    let clip = d.add_layer("クリップ").unwrap();
    paint(d, clip, &mut rng, &[255, 60]);
    d.set_layer_blend_mode(clip, BlendMode::Screen).unwrap();
    d.set_layer_clipping(clip, true).unwrap();
    let fill = d
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(30, 90, 220, 140))],
            None,
        )
        .unwrap();
    d.set_layer_blend_mode(fill, BlendMode::Overlay).unwrap();
    d.add_layer_mask(fill).unwrap();
    for y in 0..d.height() / 2 {
        for x in 0..d.width() {
            d.set_mask_pixel(fill, x, y, 255).unwrap();
        }
    }
    let g1 = d.add_layer("組 1").unwrap();
    paint(d, g1, &mut rng, &[255, 120]);
    let g2 = d.add_layer("組 2").unwrap();
    paint(d, g2, &mut rng, &[200, 255, 0]);
    d.set_layer_clipping(g2, true).unwrap();
    d.set_channel_blend(
        g2,
        Channel::Color,
        ChannelBlend::new(Some(BlendMode::SoftLight), Some(0.7)),
        false,
    )
    .unwrap();
    d.group_layers(&[g1, g2], "組").unwrap();
}

/// ウィンドウの描いた絵の、キャンバスの矩形の中。
fn canvas_image(h: &mut Harness<'_, YoluApp>) -> (image::RgbaImage, Rect) {
    let rect = canvas_rect(h);
    (h.render().expect("描ける"), rect)
}

fn max_image_diff(a: &image::RgbaImage, b: &image::RgbaImage, rect: Rect) -> u8 {
    let mut worst = 0;
    for y in rect.top() as u32 + 1..rect.bottom() as u32 - 1 {
        for x in rect.left() as u32 + 1..rect.right() as u32 - 1 {
            let (p, q) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            for k in 0..3 {
                worst = worst.max(p[k].abs_diff(q[k]));
            }
        }
    }
    worst
}

fn screen_pixel(h: &Harness<'_, YoluApp>, image: &image::RgbaImage, x: f64, y: f64) -> [u8; 4] {
    let r = canvas_rect(h);
    let doc = &h.state().state.doc;
    let v = h.state().state.view.view(r, doc.width(), doc.height());
    let p = v.to_screen(x + 0.5, y + 0.5);
    image.get_pixel(p.x.round() as u32, p.y.round() as u32).0
}

#[test]
fn gpu_display_is_the_same_picture_as_the_cpu_display() {
    let Some(_gpu) = canvas_device::begin("gpu_display_is_the_same_picture_as_the_cpu_display")
    else {
        return;
    };
    let mut cpu = canvas_app(256, 256, CanvasBackend::Cpu);
    let mut gpu = canvas_app(256, 256, CanvasBackend::Gpu);
    rich_document(&mut cpu.state_mut().state.doc);
    rich_document(&mut gpu.state_mut().state.doc);
    cpu.run();
    gpu.run();
    assert_eq!(cpu.state().display().shown(), Shown::Cpu);
    assert_eq!(
        gpu.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        gpu.state().display().fallback()
    );
    assert_eq!(gpu.state().display().fallback(), None);
    assert!(
        gpu.state().display().page_count() == 0,
        "GPU の道は CPU の頁を持たない"
    );
    let (a, rect) = canvas_image(&mut cpu);
    let (b, _) = canvas_image(&mut gpu);
    let max = max_image_diff(&a, &b, rect);
    eprintln!("ウィンドウの絵の CPU と GPU の最大差: {max}");
    assert!(max <= 3, "最大差 {max}");
    // 絵が空でないこと（市松だけを比べていない）
    let center = screen_pixel(&gpu, &b, 100.0, 200.0);
    let checker_corner = b.get_pixel(rect.left() as u32 + 2, rect.top() as u32 + 2).0;
    assert_ne!(center, checker_corner);
    // ウィンドウの行の向き: 左下と右上で違う絵（上下を取り違えていない）
    let low = screen_pixel(&gpu, &b, 10.0, 10.0);
    let high = screen_pixel(&gpu, &b, 245.0, 245.0);
    let (cl, ch) = (
        screen_pixel(&cpu, &a, 10.0, 10.0),
        screen_pixel(&cpu, &a, 245.0, 245.0),
    );
    assert!(low[..3].iter().zip(&cl).all(|(a, b)| a.abs_diff(*b) <= 3));
    assert!(high[..3].iter().zip(&ch).all(|(a, b)| a.abs_diff(*b) <= 3));
    assert_ne!(low, high);
}

#[test]
fn stroke_updates_only_the_changed_tiles_on_gpu() {
    let Some(_gpu) = canvas_device::begin("stroke_updates_only_the_changed_tiles_on_gpu") else {
        return;
    };
    let mut h = canvas_app(1024, 1024, CanvasBackend::Gpu); // 8 × 8 = 64 タイル
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let first = h.state().display().stats;
    assert!(first.total_tiles >= 64, "初めは全部を作る: {first:?}");
    let before = first.total_tiles;
    let c = canvas_rect(&h).center();
    let start = offset(c, 30.0, 30.0);
    press(&h, start, egui::PointerButton::Primary);
    move_to(&h, offset(start, 6.0, 0.0));
    release(&h, offset(start, 6.0, 0.0), egui::PointerButton::Primary);
    h.run();
    let stats = h.state().display().stats;
    let uploaded = stats.total_tiles - before;
    assert!(!stats.last_rebuilt);
    assert!(
        (1..=4).contains(&uploaded),
        "変わったタイルだけを合成し直す: {uploaded}"
    );
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    assert_eq!(canvas_pixel(&h, start), [0, 0, 0, 255]);
    // 描いた所がウィンドウにも黒で出ている
    let image = h.render().unwrap();
    let p = image
        .get_pixel(start.x.round() as u32, start.y.round() as u32)
        .0;
    assert!(p[0] < 20 && p[1] < 20 && p[2] < 20, "{p:?}");
    // 取り消すと戻る
    key(&h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run();
    let image = h.render().unwrap();
    let p = image
        .get_pixel(start.x.round() as u32, start.y.round() as u32)
        .0;
    assert!(p[0] > 100, "取消のあと市松に戻る: {p:?}");
}

fn dab_color(image: &image::RgbaImage, at: Pos2) -> [u8; 4] {
    image.get_pixel(at.x.round() as u32, at.y.round() as u32).0
}

/// 効果（フィルター・Generator・塗りつぶしのグラデーションと投影・マスクの効果）のある文書も GPU で合成する。効果の出力は CPU（core）が
/// 評価して、タイルとして GPU へ上げるので、ウィンドウの絵に効果が入る。効果を外すと、その絵に戻る。
#[test]
fn documents_with_effects_stay_on_the_gpu_and_show_the_effects() {
    let Some(_gpu) =
        canvas_device::begin("documents_with_effects_stay_on_the_gpu_and_show_the_effects")
    else {
        return;
    };
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let mut h = canvas_app(128, 128, CanvasBackend::Gpu);
    let base = h.state().state.doc.layers()[0].id();
    for y in 0..128 {
        for x in 0..128 {
            h.state_mut()
                .state
                .doc
                .set_pixel(base, x, y, Rgba8::new(200, 30, 60, 255))
                .unwrap();
        }
    }
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let at = canvas_rect(&h).center();
    // 反転のフィルターを足しても GPU のまま、ウィンドウの絵は反転した色
    let filter = h
        .state_mut()
        .state
        .doc
        .add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
        )
        .unwrap();
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    assert_eq!(h.state().display().fallback(), None);
    let image = h.render().unwrap();
    assert_eq!(
        &dab_color(&image, at)[..3],
        &[55, 225, 195],
        "フィルターが表示に入る"
    );
    // 無効にする・有効に戻すと、その絵になる（GPU のまま、変わったタイルを上げ直す）
    h.state_mut()
        .state
        .doc
        .set_filter_enabled(base, filter, false)
        .unwrap();
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let image = h.render().unwrap();
    assert_eq!(&dab_color(&image, at)[..3], &[200, 30, 60]);
    h.state_mut()
        .state
        .doc
        .set_filter_enabled(base, filter, true)
        .unwrap();
    h.run();
    let image = h.render().unwrap();
    assert_eq!(&dab_color(&image, at)[..3], &[55, 225, 195]);
    // マスクの効果も同じ（マスクの値が評価で決まる）
    h.state_mut().state.doc.add_layer_mask(base).unwrap();
    h.state_mut()
        .state
        .doc
        .add_filter(
            base,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(2)),
        )
        .unwrap();
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    assert_eq!(h.state().display().fallback(), None);
}

/// ウィンドウの絵で、効果のある文書の GPU の表示が CPU の表示と同じ絵になる。
#[test]
fn effects_adjustments_and_isolated_groups_show_the_same_picture_on_the_gpu_and_the_cpu() {
    let Some(_gpu) = canvas_device::begin(
        "effects_adjustments_and_isolated_groups_show_the_same_picture_on_the_gpu_and_the_cpu",
    ) else {
        return;
    };
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let mut cpu = canvas_app(256, 256, CanvasBackend::Cpu);
    let mut gpu = canvas_app(256, 256, CanvasBackend::Gpu);
    for h in [&mut cpu, &mut gpu] {
        let d = &mut h.state_mut().state.doc;
        rich_document(d);
        // 効果のあるレイヤー・調整レイヤー・独立して合成するグループ・法線の種類のチャンネルを重ねる
        let top = d.layers().last().unwrap().id();
        let mut rng = Rng(77);
        let blurred = d.add_layer("ぼかす").unwrap();
        paint(d, blurred, &mut rng, &[255, 0, 0, 160]);
        d.add_filter(
            blurred,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .unwrap();
        let adj = d
            .add_adjustment_layer(
                "色相",
                AdjustmentSettings::hue_saturation(50.0, 0.2, 0.0).unwrap(),
                None,
                None,
            )
            .unwrap();
        d.set_layer_opacity(adj, 0.7, false).unwrap();
        let inner = d.add_layer("組の中").unwrap();
        paint(d, inner, &mut rng, &[255, 120]);
        d.set_layer_blend_mode(inner, BlendMode::Multiply).unwrap();
        let group = d.group_layers(&[inner], "独立の組").unwrap();
        d.set_layer_blend_mode(group, BlendMode::Normal).unwrap();
        d.set_layer_opacity(group, 0.8, false).unwrap();
        let _ = top;
    }
    cpu.run();
    gpu.run();
    assert_eq!(cpu.state().display().shown(), Shown::Cpu);
    assert_eq!(
        gpu.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        gpu.state().display().fallback()
    );
    let (a, rect) = canvas_image(&mut cpu);
    let (b, _) = canvas_image(&mut gpu);
    let max = max_image_diff(&a, &b, rect);
    eprintln!("ウィンドウの絵の CPU と GPU の最大差（効果・調整・独立のグループ）: {max}");
    // 多段の文書の許し（文書の説明と同じ 2）
    assert!(max <= 2, "最大差 {max}");
}

/// 効果のある文書を、全タイルの上限の見積もりが予算を超えるとき CPU で表示し、収まれば GPU へ戻る（効果は CPU の表示でも見える）。
#[test]
fn effects_over_the_budget_use_the_cpu_and_come_back() {
    let Some(_gpu) = canvas_device::begin("effects_over_the_budget_use_the_cpu_and_come_back")
    else {
        return;
    };
    let mut h = canvas_app(512, 512, CanvasBackend::Cpu);
    h.state_mut().set_canvas_gpu_budget(3 << 20);
    h.state_mut().set_canvas_backend(CanvasBackend::Gpu);
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "空の文書は予算に収まる"
    );
    // ノイズのフィルターを掛けた塗りつぶし: 16 タイルがどれも評価の出力を持ち得るので、全部を常駐させると予算を超える
    use yolu_core::{EffectSettings, FilterSpec, FilterTarget};
    let fill = h
        .state_mut()
        .state
        .doc
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(128, 128, 128, 255))],
            None,
        )
        .unwrap();
    let noise = h
        .state_mut()
        .state
        .doc
        .add_filter(
            fill,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::noise(0.8, 3, false)).channels(&[Channel::Color]),
        )
        .unwrap();
    h.run();
    assert_eq!(h.state().display().fallback(), Some(&Fallback::OverBudget));
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    assert_eq!(
        h.state().display().gpu().failures,
        0,
        "GPU を試して落ちたのではなく、見積もりで選んだ"
    );
    // CPU の表示にもノイズが見える（2 点が同じ灰色でない絵）
    let image = h.render().unwrap();
    let (p, q) = (
        screen_pixel(&h, &image, 20.0, 20.0),
        screen_pixel(&h, &image, 490.0, 490.0),
    );
    assert_ne!(p, q, "単色でない");
    // 効果を外すと、予算に収まって GPU へ戻る
    h.state_mut().state.doc.remove_filter(fill, noise).unwrap();
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    assert_eq!(h.state().display().fallback(), None);
}

/// グループの入れ子が GPU の積みの深さを超える文書だけが、理由つきで CPU に落ちる。調整レイヤー・独立のグループ・法線のチャンネルは GPU のまま。
#[test]
fn only_too_deep_groups_fall_back_with_a_reason_and_come_back() {
    let Some(_gpu) =
        canvas_device::begin("only_too_deep_groups_fall_back_with_a_reason_and_come_back")
    else {
        return;
    };
    let mut h = canvas_app(128, 128, CanvasBackend::Gpu);
    let base = h.state().state.doc.layers()[0].id();
    for y in 0..128 {
        for x in 0..128 {
            h.state_mut()
                .state
                .doc
                .set_pixel(base, x, y, Rgba8::new(200, 30, 60, 255))
                .unwrap();
        }
    }
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let at = canvas_rect(&h).center();
    let image = h.render().unwrap();
    assert_eq!(&dab_color(&image, at)[..3], &[200, 30, 60]);
    // 調整レイヤーは GPU で合成する（反転した色）
    let adj = h
        .state_mut()
        .state
        .doc
        .add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    assert_eq!(h.state().display().fallback(), None);
    assert_eq!(h.state().display().page_count(), 0);
    let image = h.render().unwrap();
    assert_eq!(&dab_color(&image, at)[..3], &[55, 225, 195]);
    h.state_mut().state.doc.remove_layer(adj).unwrap();
    // 独立して合成するグループも GPU
    let l2 = h.state_mut().state.doc.add_layer("上").unwrap();
    let group = h.state_mut().state.doc.group_layers(&[l2], "組").unwrap();
    h.state_mut()
        .state
        .doc
        .set_layer_blend_mode(group, BlendMode::Multiply)
        .unwrap();
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    // 法線のチャンネルを見ても GPU（法線は単位ベクトルの合成式）
    h.state_mut().state.m2.display_channel = Channel::Normal;
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    let total = h.state().display().stats.total_tiles;
    h.state_mut().state.m2.display_channel = Channel::Roughness;
    h.step();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "別のチャンネルは GPU で合成し直す"
    );
    // 替えた直後のフレームの記録（あとのフレームは「何も上げない」で上書きする）と、積み上がる数
    assert!(
        h.state().display().stats.last_rebuilt,
        "チャンネルを替えると作り直す"
    );
    assert!(h.state().display().stats.total_tiles > total);
    h.state_mut().state.m2.display_channel = Channel::Color;
    h.run();
    let transitions = h.state().display().stats.total_tiles;
    // 入れ子を深くする（独立して合成するグループ 33 段）: GPU は断り、CPU の表示になる
    let mut inner = h.state_mut().state.doc.add_layer("葉").unwrap();
    for k in 0..33 {
        let g = h
            .state_mut()
            .state
            .doc
            .group_layers(&[inner], &format!("段 {k}"))
            .unwrap();
        h.state_mut()
            .state
            .doc
            .set_layer_blend_mode(g, BlendMode::Multiply)
            .unwrap();
        inner = g;
    }
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    assert_eq!(
        h.state().display().fallback(),
        Some(&Fallback::Unsupported(Unsupported::GroupDepth))
    );
    for lang in Lang::ALL {
        let text = h.state().display().fallback().unwrap().describe(lang);
        assert_eq!(has_japanese(&text), lang == Lang::Ja, "{text}");
    }
    assert!(h.state().display().page_count() > 0);
    assert!(
        !h.state().display().gpu().is_resident(),
        "GPU の資源を手放す"
    );
    // 同じ文書のまま何フレームか回しても、GPU を試し直さない（断りは文書の版ごとの確認だけ）
    let failures = h.state().display().gpu().failures;
    for _ in 0..30 {
        h.step();
    }
    assert_eq!(h.state().display().gpu().failures, failures);
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    // 深いグループを隠すと GPU に戻る
    h.state_mut()
        .state
        .doc
        .set_layer_visible(inner, false)
        .unwrap();
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    assert_eq!(h.state().display().fallback(), None);
    assert_eq!(h.state().display().page_count(), 0, "CPU の頁を手放す");
    assert!(h.state().display().stats.total_tiles > transitions);
}

/// 512² の文書に 128² のタイルを足していく。予算 3 MiB では、表示 1 MiB・作業域の分を除くと、描いたタイルは 8 枚ほどまで常駐できる。
fn put_tile(h: &mut Harness<'_, YoluApp>, layer: LayerId, index: u32, filled: bool) {
    let bytes = if filled {
        [90u8, 160, 220, 255].repeat(128 * 128)
    } else {
        vec![0u8; 128 * 128 * 4]
    };
    h.state_mut()
        .state
        .doc
        .import_tile(
            layer,
            Channel::Color,
            yolu_app::engine::TileCoord::new(index % 4, index / 4),
            &bytes,
        )
        .unwrap();
    h.run();
}

#[test]
fn documents_over_the_budget_use_the_cpu_and_come_back_with_a_margin() {
    let Some(_gpu) =
        canvas_device::begin("documents_over_the_budget_use_the_cpu_and_come_back_with_a_margin")
    else {
        return;
    };
    let mut h = canvas_app(512, 512, CanvasBackend::Cpu);
    let layer = h.state().state.doc.layers()[0].id();
    h.state_mut().set_canvas_gpu_budget(3 << 20);
    h.state_mut().set_canvas_backend(CanvasBackend::Gpu);
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "空の文書は予算に収まる"
    );
    // 描いたタイルが増えて、全部を常駐させると予算を超えるところで CPU へ
    let mut crossed = None;
    for index in 0..16 {
        put_tile(&mut h, layer, index, true);
        if crossed.is_none() && h.state().display().fallback() == Some(&Fallback::OverBudget) {
            crossed = Some(index + 1);
        }
        assert_eq!(
            h.state().display().shown(),
            if crossed.is_some() {
                Shown::Cpu
            } else {
                Shown::Gpu
            },
            "{index}"
        );
    }
    let crossed = crossed.expect("16 枚では予算を超える");
    eprintln!("予算 3 MiB の 512² で、{crossed} 枚目のタイルで CPU へ");
    assert!((6..=11).contains(&crossed), "{crossed}");
    assert!(h.state().display().page_count() > 0);
    assert_eq!(
        h.state().display().gpu().failures,
        0,
        "GPU を試して落ちたのではなく、見積もりで選んだ"
    );
    assert!(
        !h.state().display().gpu().is_resident(),
        "GPU の資源は持たない"
    );
    // CPU の表示が、描いたタイルをそのまま見せている
    let image = h.render().unwrap();
    assert_eq!(&screen_pixel(&h, &image, 10.0, 10.0)[..3], &[90, 160, 220]);
    // 1 枚減らしても、まだ予算の 8 割より大きいので戻らない（境で行き来しない）
    put_tile(&mut h, layer, 15, false);
    assert_eq!(h.state().display().fallback(), Some(&Fallback::OverBudget));
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    // 8 割に収まるまで減らすと GPU に戻る
    let mut back = None;
    for index in (0..15).rev() {
        put_tile(&mut h, layer, index, false);
        if h.state().display().shown() == Shown::Gpu {
            back = Some(index);
            break;
        }
    }
    let back = back.expect("タイルを減らせば戻る");
    assert!(
        back < crossed - 1,
        "戻るのは、超えたときより少なくなってから: 戻った {back} / 超えた {crossed}"
    );
    assert_eq!(h.state().display().page_count(), 0);
    assert_eq!(h.state().display().fallback(), None);
    // 予算を広げれば、描いたタイルが多くても GPU
    for index in 0..16 {
        put_tile(&mut h, layer, index, true);
    }
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    h.state_mut()
        .set_canvas_gpu_budget(yolu_app::canvas::gpu::RESIDENT_BUDGET);
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
}

#[test]
fn a_budget_too_small_for_the_display_falls_back_without_trying_the_gpu() {
    let Some(_gpu) = canvas_device::begin(
        "a_budget_too_small_for_the_display_falls_back_without_trying_the_gpu",
    ) else {
        return;
    };
    let mut h = canvas_app(1024, 1024, CanvasBackend::Cpu);
    h.state_mut().set_canvas_gpu_budget(1 << 20); // 表示のテクスチャ（4 MiB）が入らない
    h.state_mut().set_canvas_backend(CanvasBackend::Gpu);
    h.run();
    assert_eq!(h.state().display().fallback(), Some(&Fallback::OverBudget));
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    assert!(h.state().display().page_count() > 0);
    assert_eq!(h.state().display().gpu().failures, 0);
    // 描いても CPU の表示が更新される
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 40.0, 0.0)]);
    assert!(h.state().state.doc.can_undo());
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    assert_eq!(h.state().display().gpu().failures, 0);
}

#[test]
fn texture_limits_fall_back_after_one_failed_try_and_retry_only_when_the_document_changes() {
    let Some(_gpu) = canvas_device::begin(
        "texture_limits_fall_back_after_one_failed_try_and_retry_only_when_the_document_changes",
    ) else {
        return;
    };
    // 表示のテクスチャがデバイスの上限（既定 8192）を超える文書は、CPU の頁（2048 ごと）で見せる
    let mut wide = canvas_app(9000, 64, CanvasBackend::Gpu);
    // 実際の yolu-gpu の失敗（日本語の文）を、種類で受ける。英語の画面には日本語を出さない
    let failure = match wide.state().display().fallback() {
        Some(Fallback::Failed(failure)) => failure.clone(),
        other => panic!("上限で断る: {other:?}"),
    };
    assert_eq!(
        failure.kind,
        FailureKind::TextureLimit,
        "{}",
        failure.detail
    );
    assert!(failure.detail.contains("上限"), "{}", failure.detail);
    let reason = Fallback::Failed(failure);
    assert!(
        !has_japanese(&reason.describe(Lang::En)),
        "{}",
        reason.describe(Lang::En)
    );
    assert!(has_japanese(&reason.describe(Lang::Ja)));
    assert_eq!(wide.state().display().shown(), Shown::Cpu);
    assert_eq!(wide.state().display().page_count(), 5, "9000 / 2048 の頁");
    assert_eq!(wide.state().display().gpu().failures, 1);
    for _ in 0..30 {
        wide.step();
    }
    assert_eq!(
        wide.state().display().gpu().failures,
        1,
        "毎フレーム試さない"
    );
    // 描いても試し直さない
    let c = canvas_rect(&wide).center();
    drag(&mut wide, &[c, offset(c, 40.0, 0.0)]);
    assert_eq!(wide.state().display().gpu().failures, 1);
    // レイヤーの数が変わると 1 回試し直す（同じ理由でまた CPU）
    wide.state_mut().state.doc.add_layer("もう 1 枚").unwrap();
    wide.run();
    assert_eq!(wide.state().display().gpu().failures, 2);
    assert_eq!(wide.state().display().shown(), Shown::Cpu);
}

#[test]
fn auto_uses_the_gpu_except_on_software_adapters() {
    let Some(_gpu) = canvas_device::begin("auto_uses_the_gpu_except_on_software_adapters") else {
        return;
    };
    let h = canvas_app(128, 128, CanvasBackend::Auto);
    let info = h
        .state()
        .display()
        .gpu()
        .adapter_info()
        .expect("装置がある");
    eprintln!(
        "Auto の試験のアダプター: {} ({:?})",
        info.name, info.device_type
    );
    if info.device_type == wgpu_device_type_cpu() {
        assert_eq!(h.state().display().shown(), Shown::Cpu);
        assert_eq!(
            h.state().display().fallback(),
            Some(&Fallback::SoftwareAdapter)
        );
    } else {
        assert_eq!(h.state().display().shown(), Shown::Gpu);
    }
    // 方針が CPU なら常に CPU
    let h = canvas_app(128, 128, CanvasBackend::Cpu);
    assert_eq!(h.state().display().fallback(), Some(&Fallback::Policy));
}

fn wgpu_device_type_cpu() -> eframe::egui_wgpu::wgpu::DeviceType {
    eframe::egui_wgpu::wgpu::DeviceType::Cpu
}

#[test]
fn zoom_switches_the_sampler_without_rebuilding() {
    let Some(_gpu) = canvas_device::begin("zoom_switches_the_sampler_without_rebuilding") else {
        return;
    };
    // 1024² はウィンドウに収めると画素が 1 点より小さい（補間する）。拡大して 1 画素が 2 点を超えると画素の角を見せる。
    let mut h = canvas_app(1024, 1024, CanvasBackend::Gpu);
    let base = h.state().state.doc.layers()[0].id();
    for y in 500..510 {
        for x in 500..510 {
            h.state_mut()
                .state
                .doc
                .set_pixel(base, x, y, Rgba8::new(255, 0, 0, 255))
                .unwrap();
        }
    }
    h.run();
    assert!(!h.state().display().is_nearest());
    let id = h.state().display().gpu().texture().map(|t| t.0).unwrap();
    let total = h.state().display().stats.total_tiles;
    h.state_mut().state.view.zoom = 8.0;
    h.run();
    assert!(
        h.state().display().is_nearest(),
        "拡大すると画素の角を見せる"
    );
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    assert_eq!(
        h.state().display().gpu().texture().map(|t| t.0),
        Some(id),
        "テクスチャの id は同じ（登録し直しだけ）"
    );
    assert_eq!(
        h.state().display().stats.total_tiles,
        total,
        "補間の切り替えで作り直さない"
    );
    // 拡大した赤い画素が赤く見える
    let r = canvas_rect(&h);
    let v = h.state().state.view.view(r, 1024, 1024);
    let p = v.to_screen(505.0, 505.0);
    assert!(r.contains(p), "{p:?} {r:?}");
    let image = h.render().unwrap();
    let px = image.get_pixel(p.x.round() as u32, p.y.round() as u32).0;
    assert!(
        px[0] > 240 && px[1] < 10 && px[2] < 10,
        "拡大した画素: {px:?}"
    );
    h.state_mut().state.view.zoom = 1.0;
    h.run();
    assert!(!h.state().display().is_nearest());
    assert_eq!(h.state().display().stats.total_tiles, total);
}

#[test]
fn replacing_the_document_rebuilds_the_gpu_display() {
    let Some(_gpu) = canvas_device::begin("replacing_the_document_rebuilds_the_gpu_display") else {
        return;
    };
    let mut h = canvas_app(128, 128, CanvasBackend::Gpu);
    let before = h.state().display().stats;
    let mut doc = Document::new(96, 64).unwrap();
    let l = doc.add_layer("新しい文書").unwrap();
    for y in 0..64 {
        for x in 0..96 {
            doc.set_pixel(l, x, y, Rgba8::new(10, 200, 30, 255))
                .unwrap();
        }
    }
    h.state_mut().state.doc = doc;
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let after = h.state().display().stats;
    assert!(after.total_tiles > before.total_tiles);
    let image = h.render().unwrap();
    assert_eq!(&screen_pixel(&h, &image, 40.0, 30.0)[..3], &[10, 200, 30]);
}

/// 試験用の一時の置き場（target の中。プロセスごとに分ける）。
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/canvas-gpu-tests")
        .join(std::process::id().to_string())
        .join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 不変条件: GPU の道は正本を読むだけで書かない。GPU が表示している間に何フレーム回し、表示の操作（拡大・チャンネル・方針の
/// 切り替え）をしても、文書の版・変更記録・画素・取消の履歴は変わらず、書き出し（合成）も保存（.ylp のバイト）も、CPU の表示の
/// ときと同じ正本から作られる。
#[test]
fn the_gpu_display_never_writes_the_document_and_saving_is_the_same_as_on_the_cpu() {
    let Some(_gpu) = canvas_device::begin(
        "the_gpu_display_never_writes_the_document_and_saving_is_the_same_as_on_the_cpu",
    ) else {
        return;
    };
    let mut h = canvas_app(256, 256, CanvasBackend::Gpu);
    rich_document(&mut h.state_mut().state.doc);
    // 描いた 1 本のストロークも、取消の履歴に 1 つ残す
    let c = canvas_rect(&h).center();
    drag(&mut h, &[c, offset(c, 40.0, 10.0)]);
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    let snapshot = |h: &Harness<'_, YoluApp>| {
        let doc = &h.state().state.doc;
        (
            doc.revision(),
            doc.change_serial(),
            doc.composite(doc.bounds()).unwrap(),
            doc.can_undo(),
            doc.can_redo(),
            doc.history_bytes(),
            doc.allocated_bytes(),
            doc.layers().len(),
        )
    };
    let before = snapshot(&h);
    assert!(before.3, "ストロークの取消が残っている");
    // 表示だけの操作: 何フレームか回す・拡大して戻す・チャンネルを見て戻す（法線で CPU へ落ちて GPU へ戻る）
    for _ in 0..20 {
        h.step();
    }
    h.state_mut().state.view.zoom = 6.0;
    h.run();
    h.state_mut().state.view.zoom = 1.0;
    h.run();
    for channel in [Channel::Roughness, Channel::Normal, Channel::Color] {
        h.state_mut().state.m2.display_channel = channel;
        h.run();
    }
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let _ = h.render().unwrap();
    assert!(
        before == snapshot(&h),
        "GPU の表示のあいだ、文書は何も変わらない"
    );
    // 保存は GPU が表示しているときも、CPU に切り替えたあとも同じバイト（どちらも CPU の正本から書く）
    let dir = scratch("never-writes");
    let (on_gpu, on_cpu) = (dir.join("gpu.ylp"), dir.join("cpu.ylp"));
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(on_gpu.clone()));
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    h.state_mut().set_canvas_backend(CanvasBackend::Cpu);
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Cpu);
    assert!(before == snapshot(&h), "方針を替えても文書は変わらない");
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(on_cpu.clone()));
    assert_eq!(
        std::fs::read(&on_gpu).unwrap(),
        std::fs::read(&on_cpu).unwrap(),
        "保存した .ylp は表示の道によらない"
    );
    // 取消も GPU の表示と無関係に効く（GPU に戻して取り消すと、描く前の正本に戻る）
    h.state_mut().set_canvas_backend(CanvasBackend::Gpu);
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    key(&h, egui::Key::Z, egui::Modifiers::COMMAND);
    h.run();
    assert!(h.state().state.doc.revision() > before.0);
    let after_undo = h
        .state()
        .state
        .doc
        .composite(h.state().state.doc.bounds())
        .unwrap();
    assert_ne!(after_undo, before.2, "取消で正本が戻る");
    let _ = std::fs::remove_dir_all(dir);
}

/// 同じ文書 ID の別の中身に替わったとき（保存した ID が戻る .ylp の読み直し・PSD の読み直し）、前の文書の合成を差分の土台にしない。
/// 読み直した文書の変更記録の通し番号は、前より小さいことも大きいこともあるので、どちらでも新しい絵になる。
fn same_id_reload(policy: CanvasBackend) {
    let mut h = canvas_app(256, 256, policy);
    let shown = if policy == CanvasBackend::Gpu {
        Shown::Gpu
    } else {
        Shown::Cpu
    };
    let id = h.state().state.doc.id();
    let base = h.state().state.doc.layers()[0].id();
    // 1. 保存して、そのあとたくさん描く（通し番号が保存した文書より大きくなる）→ 開き直す（通し番号が小さい読み直し）
    for x in 0..8 {
        h.state_mut()
            .state
            .doc
            .set_pixel(base, 5 + x, 5, Rgba8::new(255, 0, 0, 255))
            .unwrap();
    }
    let dir = scratch(if policy == CanvasBackend::Gpu {
        "reload-gpu"
    } else {
        "reload-cpu"
    });
    let path = dir.join("p.ylp");
    h.state_mut()
        .state
        .apply(Action::SaveProjectAs(path.clone()));
    assert!(
        h.state().state.message.starts_with("保存しました"),
        "{}",
        h.state().state.message
    );
    for y in 130..180 {
        for x in 130..180 {
            h.state_mut()
                .state
                .doc
                .set_pixel(base, x, y, Rgba8::new(0, 255, 0, 255))
                .unwrap();
        }
    }
    h.run();
    assert_eq!(h.state().display().shown(), shown);
    let served_before = h.state().state.doc.change_serial();
    let image = h.render().unwrap();
    assert_eq!(&screen_pixel(&h, &image, 150.0, 150.0)[..3], &[0, 255, 0]);
    h.state_mut().state.apply(Action::OpenProject(path.clone()));
    assert!(
        h.state().state.message.starts_with("開きました"),
        "{}",
        h.state().state.message
    );
    h.run();
    assert_eq!(
        h.state().state.doc.id(),
        id,
        "読み直しは保存した文書 ID が戻る"
    );
    assert!(
        h.state().state.doc.change_serial() < served_before,
        "読み直した文書の通し番号は小さい"
    );
    assert_eq!(
        h.state().display().shown(),
        shown,
        "{:?}",
        h.state().display().fallback()
    );
    assert_eq!(
        h.state().display().gpu().failures,
        0,
        "世代の巻き戻りを GPU の失敗にしない"
    );
    // 「開きました」の知らせがキャンバスの左下の隅に重なるので、読む前に消す
    h.state_mut().state.clear_message();
    h.run();
    let image = h.render().unwrap();
    assert_eq!(
        &screen_pixel(&h, &image, 7.0, 5.0)[..3],
        &[255, 0, 0],
        "保存した赤"
    );
    assert_ne!(
        &screen_pixel(&h, &image, 150.0, 150.0)[..3],
        &[0, 255, 0],
        "保存のあとに描いた緑は残らない"
    );
    // 2. 通し番号が前より大きい同じ ID の文書（差分で読むと、前の文書の絵が一部のタイルに残る）。読み直した文書にまず編集を重ねて
    //    通し番号を上げ、それより大きくなる別の中身を作る。早いうちに描いた左下のタイルは、表示済みの通し番号より前の記録になる
    let base = h.state().state.doc.layers()[0].id();
    for k in 0..100 {
        h.state_mut()
            .state
            .doc
            .set_pixel(
                base,
                200 + k % 50,
                200 + k / 50,
                Rgba8::new(90, 90, 90, 255),
            )
            .unwrap();
    }
    h.run();
    let shown_serial = h.state().state.doc.change_serial();
    let mut other = Document::new(256, 256).unwrap();
    let layer = other.add_layer("別の中身").unwrap();
    let mut other = other.with_persistent_ids(id, &[layer]).unwrap();
    for x in 0..8 {
        other
            .set_pixel(layer, 5 + x, 5, Rgba8::new(0, 0, 255, 255))
            .unwrap();
    }
    for k in 0..u32::MAX {
        other
            .set_pixel(
                layer,
                130 + k % 120,
                (k / 120) % 100,
                Rgba8::new(255, 255, 0, 255),
            )
            .unwrap();
        if other.change_serial() > shown_serial + 4 {
            break;
        }
    }
    let sets = yolu_app::sets::TextureSets::first_in(&other, Lang::Ja);
    h.state_mut().state.replace_sets(sets, other);
    h.run();
    assert_eq!(h.state().state.doc.id(), id);
    assert!(
        h.state().state.doc.change_serial() > shown_serial,
        "通し番号は大きい"
    );
    assert_eq!(
        h.state().display().shown(),
        shown,
        "{:?}",
        h.state().display().fallback()
    );
    let image = h.render().unwrap();
    assert_eq!(
        &screen_pixel(&h, &image, 7.0, 5.0)[..3],
        &[0, 0, 255],
        "差分で読まず、新しい文書の絵になる（前の文書の絵が残らない）"
    );
    assert_eq!(&screen_pixel(&h, &image, 142.0, 0.0)[..3], &[255, 255, 0]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn reloading_a_document_with_the_same_id_rebuilds_the_gpu_display() {
    let Some(_gpu) =
        canvas_device::begin("reloading_a_document_with_the_same_id_rebuilds_the_gpu_display")
    else {
        return;
    };
    same_id_reload(CanvasBackend::Gpu);
}

#[test]
fn reloading_a_document_with_the_same_id_rebuilds_the_cpu_display() {
    let Some(_gpu) =
        canvas_device::begin("reloading_a_document_with_the_same_id_rebuilds_the_cpu_display")
    else {
        return;
    };
    same_id_reload(CanvasBackend::Cpu);
}

/// 文書を直に入れ替えても（`replace_sets` を通さない）、変更記録の通し番号が戻っていれば、GPU の失敗にせず作り直す。
#[test]
fn a_rolled_back_change_record_rebuilds_instead_of_failing() {
    let Some(_gpu) =
        canvas_device::begin("a_rolled_back_change_record_rebuilds_instead_of_failing")
    else {
        return;
    };
    let mut h = canvas_app(256, 256, CanvasBackend::Gpu);
    let id = h.state().state.doc.id();
    let base = h.state().state.doc.layers()[0].id();
    for y in 0..60 {
        h.state_mut()
            .state
            .doc
            .set_pixel(base, 3, y, Rgba8::new(255, 0, 0, 255))
            .unwrap();
    }
    h.run();
    assert_eq!(h.state().display().shown(), Shown::Gpu);
    let mut other = Document::new(256, 256).unwrap();
    let layer = other.add_layer("別の中身").unwrap();
    let mut other = other.with_persistent_ids(id, &[layer]).unwrap();
    other
        .set_pixel(layer, 3, 3, Rgba8::new(0, 0, 255, 255))
        .unwrap();
    assert!(other.change_serial() < h.state().state.doc.change_serial());
    h.state_mut().state.doc = other;
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    assert_eq!(h.state().display().gpu().failures, 0);
    let image = h.render().unwrap();
    assert_eq!(&screen_pixel(&h, &image, 3.0, 3.0)[..3], &[0, 0, 255]);
}

/// 乗算済みの表示のテクスチャが、egui の `Color32::from_rgba_unmultiplied`（本物。egui を上げて丸めが変わると落ちる）と、
/// 値 256 × アルファ 256 の全組合せでバイトまで一致する。1 レイヤーの通常の合成は、透明の上ではそのままの画素になる。
#[test]
fn the_gpu_display_texture_matches_egui_premultiplication_for_every_value_and_alpha() {
    let Some(_gpu) = canvas_device::begin(
        "the_gpu_display_texture_matches_egui_premultiplication_for_every_value_and_alpha",
    ) else {
        return;
    };
    let mut h = canvas_app(256, 256, CanvasBackend::Gpu);
    let base = h.state().state.doc.layers()[0].id();
    // 横が値、縦がアルファ。3 つのチャンネルがそれぞれ 0〜255 の全部を通る
    for y in 0..256u32 {
        for x in 0..256u32 {
            let (r, g, b) = (x as u8, 255 - x as u8, (x as u8).wrapping_mul(37));
            h.state_mut()
                .state
                .doc
                .set_pixel(base, x, y, Rgba8::new(r, g, b, y as u8))
                .unwrap();
        }
    }
    h.run();
    assert_eq!(
        h.state().display().shown(),
        Shown::Gpu,
        "{:?}",
        h.state().display().fallback()
    );
    let bounds = DocRect::new(0, 0, 256, 256);
    let straight = h.state().state.doc.composite(bounds).unwrap();
    let texture = h.state_mut().read_canvas_gpu_display(bounds).unwrap();
    assert_eq!(texture.len(), straight.len());
    let mut partial = 0;
    for (i, (s, t)) in straight
        .as_chunks::<4>()
        .0
        .iter()
        .zip(texture.as_chunks::<4>().0)
        .enumerate()
    {
        let expected = egui::Color32::from_rgba_unmultiplied(s[0], s[1], s[2], s[3]).to_array();
        if s[3] != 0 && s[3] != 255 {
            partial += 1;
        }
        assert_eq!(
            *t,
            expected,
            "x={} alpha={} straight={s:?}",
            i % 256,
            i / 256
        );
    }
    assert!(partial > 60_000, "半透明の組合せを通した: {partial}");
}

/// WebGL2 の上限で作った装置（egui-wgpu が OpenGL のときに作る形。storage の入れ物も compute も無い）では合成のシェーダーを動かせない。
/// 理由は装置の上限で、予算の超過（OverBudget）ではない（束の入れ物を作れず、要る量の見積もりが予算を超えて見えていた）。
#[test]
fn a_device_without_compute_falls_back_with_the_device_reason() {
    use eframe::egui_wgpu::{wgpu, WgpuSetup};
    use yolu_app::canvas::gpu::GpuCanvas;
    let Some(_gpu) =
        canvas_device::begin("a_device_without_compute_falls_back_with_the_device_reason")
    else {
        return;
    };
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let WgpuSetup::CreateNew(create) = &mut setup {
        create.device_descriptor = std::sync::Arc::new(|adapter| wgpu::DeviceDescriptor {
            label: Some("WebGL2 の上限"),
            required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                .using_resolution(adapter.limits()),
            ..Default::default()
        });
    }
    let rs = egui_kittest::wgpu::create_render_state(setup, render_options());
    assert!(!yolu_gpu::device_can_composite(&rs.device.limits()));
    let mut canvas = GpuCanvas::default();
    canvas.attach(Some(rs));
    let doc = Document::new(64, 64).unwrap();
    for policy in [CanvasBackend::Gpu, CanvasBackend::Auto] {
        assert_eq!(
            canvas.decide(policy, &doc, Channel::Color),
            Some(Fallback::DeviceLimits),
            "{policy:?}"
        );
    }
    // 方針が CPU なら、方針が理由
    assert_eq!(
        canvas.decide(CanvasBackend::Cpu, &doc, Channel::Color),
        Some(Fallback::Policy)
    );
}
