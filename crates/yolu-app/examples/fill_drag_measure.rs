//! 塗りつぶしレイヤーの投影の置き場をドラッグしている間（文書が変更をまとめている間）の、3D ビューの絵（`view3d::paint::Paint`）の
//! 同期 1 回の時間を測る台。試しの立方体の位置と法線のマップを CPU で焼き、トライプラナーで画像を投げた塗りつぶしレイヤーを 1 枚置いて、
//! 置き場の中心を少しずつ動かしながら `Paint::sync`（合成・上げる・ミップ）を GPU の積みが終わるまで測る。離したあとの 1 回も測る。
//!
//! 使い方: `cargo run -p yolu-app --example fill_drag_measure -- [辺 4096] [回 8]`
//! 出力: `段<TAB>中央値(ms)<TAB>各回(ms)`。マップを焼く時間・最初の全面の同期は別の行。

use std::time::Instant;

use eframe::egui_wgpu::wgpu;
use yolu_app::bake::{BakeAction, BakeBackend};
use yolu_app::fillfx::{inputs, FillOp};
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::state::{Action, AppState};
use yolu_app::view3d::paint::Paint;
use yolu_core::fill_image::ProjectionMode;
use yolu_core::generator::MapState;
use yolu_core::mesh_maps::MeshMapKind;
use yolu_core::{Channel, MapInput};

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[s.len() / 2]
}

fn row(label: &str, v: &[f64]) {
    let each: Vec<String> = v.iter().map(|x| format!("{x:.1}")).collect();
    println!("{label}\t{:.1}\t{}", median(v), each.join(","));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let side: u32 = args.first().and_then(|a| a.parse().ok()).unwrap_or(4096);
    let rounds: usize = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(8);

    let mut s = AppState::new(side, side);
    s.bake.backend = BakeBackend::Cpu;
    s.apply(Action::LoadDemoModel);
    s.bake.settings.maps = vec![MeshMapKind::WorldNormal, MeshMapKind::Position];
    s.bake.settings.padding = 4;
    let t = Instant::now();
    s.apply(Action::Bake(BakeAction::Start));
    s.wait_bake();
    eprintln!("マップを焼いた: {:.0} ms", ms(t));
    let mut effect_inputs = s.doc.effect_inputs().clone();
    for kind in [MeshMapKind::Position, MeshMapKind::WorldNormal] {
        let map = s
            .sets
            .current()
            .mesh_maps
            .get(kind)
            .expect("焼いたマップ")
            .clone();
        effect_inputs = effect_inputs
            .with_map(MapInput::from_baked(&map, MapState::Current).expect("マップ"))
            .expect("入力");
    }
    s.doc.set_effect_inputs(effect_inputs).expect("入力を置く");

    // 1024² の模様（格子と斜めの縞）
    let n = 1024u32;
    let mut pixels = Vec::with_capacity((n * n * 4) as usize);
    for y in 0..n {
        for x in 0..n {
            let grid = (x / 64 + y / 64) % 2 == 0;
            let stripe = ((x + y) / 16) % 2 == 0;
            pixels.extend_from_slice(&[
                if grid { 220 } else { 40 },
                if stripe { 180 } else { 60 },
                (x * 255 / n) as u8,
                255,
            ]);
        }
    }
    let rid = s
        .shelf
        .add_image(Lang::Ja, "pattern", &pixels, n, n)
        .expect("棚へ入る");
    let image = inputs::image_id(&rid).expect("GUID");
    s.apply(Action::M2(Edit::NewFill));
    let layer = s.selected_layer.expect("足したレイヤー");
    s.apply(Action::Fill(FillOp::Image {
        layer,
        channel: Channel::Color,
        image: Some(image),
    }));
    let mut p = *s.doc.layer(layer).expect("レイヤー").projection();
    p.mode = ProjectionMode::Triplanar;
    s.doc.set_fill_projection(layer, p, false).expect("投影");

    let rs = egui_kittest::wgpu::create_render_state(
        egui_kittest::wgpu::default_wgpu_setup(),
        Default::default(),
    );
    let info = rs.adapter.get_info();
    eprintln!("GPU: {} ({:?})", info.name, info.backend);
    let mut paint = Paint::new(&rs);
    let sync = |paint: &mut Paint, doc: &yolu_core::Document| {
        let t = Instant::now();
        let mut encoder = rs
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        paint.sync(doc, &mut encoder);
        rs.queue.submit(Some(encoder.finish()));
        let _ = rs.device.poll(wgpu::PollType::wait_indefinitely());
        ms(t)
    };
    row("first", &[sync(&mut paint, &s.doc)]);

    let mut drags = Vec::new();
    let mut releases = Vec::new();
    for k in 0..rounds {
        // 1 回のドラッグ: 3 歩動かして離す
        for step in 0..3 {
            p.placement.center[0] += 0.01 * (k * 3 + step + 1) as f64 / 10.0;
            s.doc.set_fill_projection(layer, p, true).expect("動かす");
            drags.push(sync(&mut paint, &s.doc));
        }
        s.doc.end_coalescing();
        releases.push(sync(&mut paint, &s.doc));
    }
    row("drag_step", &drags);
    row("release", &releases);
    // 内訳: 文書の合成だけ（全面・正確）と、歩幅 4 の粗い合成だけ
    let whole = s.doc.bounds();
    let coords: Vec<_> = s.doc.canvas_tiles().collect();
    let mut exact = Vec::new();
    let mut coarse = Vec::new();
    let mut buf = vec![0u8; whole.width as usize * whole.height as usize * 4];
    for _ in 0..3 {
        p.placement.center[1] += 0.003;
        s.doc.set_fill_projection(layer, p, true).expect("動かす");
        let t = Instant::now();
        s.doc
            .composite_coarse_tiles(Channel::Color, &coords, 4)
            .expect("粗い合成");
        coarse.push(ms(t));
        let t = Instant::now();
        s.doc
            .composite_into(
                Channel::Color,
                whole,
                &mut buf,
                yolu_core::RowOrder::BottomUp,
            )
            .expect("合成");
        exact.push(ms(t));
        s.doc.end_coalescing();
    }
    row("composite_exact", &exact);
    row("composite_coarse4", &coarse);
    println!("tiles\t{:?}", paint.stats);
}
