//! キャンバスの表示の GPU の合成が、CPU の合成と同じ絵になる文書の形: 独立して合成するグループ・調整の層（全種類）・法線の種類のチャンネル・
//! 効果（フィルター・Generator・塗りつぶしのグラデーション・マスクのフィルター）のある文書。
//! 許しの範囲は `tests/canvas.rs` と同じ（GPU も CPU も f32 だが、シェーダーの演算の丸めは CPU の式と同じとは限らない。層ごとに半段切り上げで丸める式は同じ。1 段ごとに最大 1、
//! 重ねた文書で 2 以内）。調整は、表を引く種類（レベル補正・トーンカーブ・明るさ/コントラスト・グラデーションマップ）と整数の式
//! （反転・2 値化・ポスタリゼーション）が CPU とバイトまで同じで、色相/彩度とカラーバランスだけ浮動小数の丸めの差が出る。
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::effects::{EffectSettings, FilterSpec, FilterTarget};
use yolu_core::generator::{self, ColorStop, OpacityStop, Ramp};
use yolu_core::{
    AdjustmentSettings, BlendMode, BrightnessContrast, Channel, ColorBalance, Document,
    GradientMap, LayerId, Posterize, Rect, Rgba8, Threshold, ToneChannel, ToneCurves,
};
use yolu_gpu::{
    resident_requirements, supports, GpuPainter, Options, ResidentCompositor, ResidentOptions,
    UpdateStats,
};

/// 表示の許し（1 画素の 1 バイトあたりの最大差）。
const TOLERANCE: u8 = 2;

#[path = "support/gpu_lease.rs"]
mod gpu_lease;

fn gpu_with(options: ResidentOptions) -> Option<ResidentCompositor> {
    gpu_lease::lease();
    let g = match GpuPainter::new(Options::default()) {
        Ok(g) => g,
        Err(e) => {
            assert!(e.to_string().starts_with("GPU 利用不可:"), "{e}");
            eprintln!("キャンバスの GPU 試験をスキップ: {e}");
            return None;
        }
    };
    Some(ResidentCompositor::with_gpu(g, options).unwrap())
}
fn gpu() -> Option<ResidentCompositor> {
    gpu_with(ResidentOptions::default())
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

/// 全面に乱数の色を置く。アルファは alphas を x で繰り返す。
fn paint_channel(d: &mut Document, layer: LayerId, channel: Channel, rng: &mut Rng, alphas: &[u8]) {
    for y in 0..d.height() {
        for x in 0..d.width() {
            let a = alphas[(x as usize + y as usize / 3) % alphas.len()];
            d.set_channel_pixel(
                layer,
                channel,
                x,
                y,
                Rgba8::new(rng.byte(), rng.byte(), rng.byte(), a),
            )
            .unwrap();
        }
    }
}
fn paint(d: &mut Document, layer: LayerId, rng: &mut Rng, alphas: &[u8]) {
    paint_channel(d, layer, Channel::Color, rng, alphas);
}

/// マスクの隠す量を全面に置く（0・端・中間をまぜる）。
fn paint_mask(d: &mut Document, layer: LayerId, rng: &mut Rng) {
    for y in 0..d.height() {
        for x in 0..d.width() {
            let hide = [0, 255, 128, rng.byte(), 1, 254][(x as usize + y as usize) % 6];
            d.set_mask_pixel(layer, x, y, hide).unwrap();
        }
    }
}

fn read(g: &mut ResidentCompositor, rect: Rect) -> Vec<u8> {
    let request = g.request_readback(rect).unwrap();
    g.finish_readback(request).unwrap()
}

fn max_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap_or(0)
}

/// 表示を更新して、CPU の合成と照らす。最大差を返す。
fn check(g: &mut ResidentCompositor, d: &Document, what: &str) -> u8 {
    check_in(g, d, Channel::Color, TOLERANCE, what)
}
fn check_in(
    g: &mut ResidentCompositor,
    d: &Document,
    channel: Channel,
    tolerance: u8,
    what: &str,
) -> u8 {
    g.update(d, channel).unwrap();
    let expected = d.composite_channel(channel, d.bounds()).unwrap();
    let actual = read(g, d.bounds());
    let max = max_diff(&expected, &actual);
    eprintln!("{what}: 最大差 {max}");
    assert!(max <= tolerance, "{what}: 最大差 {max} > {tolerance}");
    max
}
fn step(g: &mut ResidentCompositor, d: &Document, channel: Channel, what: &str) -> UpdateStats {
    let stats = g.update(d, channel).unwrap();
    let expected = d.composite_channel(channel, d.bounds()).unwrap();
    let actual = read(g, d.bounds());
    let max = max_diff(&expected, &actual);
    eprintln!(
        "{what}: 最大差 {max}、更新 {} タイル・転送 {} タイル",
        stats.updated_tiles, stats.uploaded_tiles
    );
    assert!(max <= TOLERANCE, "{what}: 最大差 {max} > {TOLERANCE}");
    stats
}

fn doc() -> Document {
    Document::with_tile_size(53, 37, 16).unwrap()
}

fn add_painted(d: &mut Document, rng: &mut Rng, alphas: &[u8]) -> LayerId {
    let l = d.add_layer("層").unwrap();
    paint(d, l, rng, alphas);
    l
}

// ───────── 疎な文書（タイルごとに流す命令を減らす） ─────────

/// タイルごとに確率 `percent`% で、そのタイルへ乱数の色を置く（置かないタイルは面に無い）。
fn paint_tiles(d: &mut Document, layer: LayerId, rng: &mut Rng, percent: u8, alphas: &[u8]) {
    let ts = d.tile_size();
    for ty in 0..d.height().div_ceil(ts) {
        for tx in 0..d.width().div_ceil(ts) {
            if u32::from(rng.byte()) * 100 / 256 >= u32::from(percent) {
                continue;
            }
            for y in ty * ts..((ty + 1) * ts).min(d.height()) {
                for x in tx * ts..((tx + 1) * ts).min(d.width()) {
                    let a = alphas[(x as usize + y as usize) % alphas.len()];
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
    }
}

/// 層が疎に描かれた文書（独立・通過のグループ・クリッピング・調整・マスクを持つ）。どのタイルも、描いた層の組み合わせが違う。
/// 描いていないタイルの命令を落としても、CPU と同じ絵になる。描き足す・消す・構造を変えると、タイルの命令も変わって追従する。
#[test]
fn sparse_documents_with_culled_tile_programs_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(509);
    let mut d = Document::with_tile_size(128, 96, 16).unwrap();
    let base = d.add_layer("下地").unwrap();
    paint_tiles(&mut d, base, &mut rng, 60, &[255]);
    let mut layers = vec![base];
    for k in 0..9 {
        let l = d.add_layer("層").unwrap();
        paint_tiles(&mut d, l, &mut rng, 35, &[255, 200, 90, 0][k % 3..]);
        d.set_layer_blend_mode(
            l,
            [
                BlendMode::Normal,
                BlendMode::Multiply,
                BlendMode::Screen,
                BlendMode::Overlay,
            ][k % 4],
        )
        .unwrap();
        layers.push(l);
    }
    check(&mut g, &d, "疎な層");
    // 一部の層だけにマスク（マスクも一部のタイルだけ）。マスクだけがあって層の画素が無いタイルもある
    for &l in &[layers[2], layers[5]] {
        d.add_layer_mask(l).unwrap();
        let ts = d.tile_size();
        for ty in 0..d.height().div_ceil(ts) {
            for tx in 0..d.width().div_ceil(ts) {
                if rng.byte().is_multiple_of(2) {
                    d.set_mask_pixel(l, tx * ts + 2, ty * ts + 3, rng.byte())
                        .unwrap();
                }
            }
        }
    }
    check(&mut g, &d, "疎なマスク");
    // クリッピング（下地が疎）と調整
    d.set_layer_clipping(layers[3], true).unwrap();
    d.set_layer_clipping(layers[4], true).unwrap();
    let adj = d
        .add_adjustment_layer(
            "色相",
            AdjustmentSettings::hue_saturation(30.0, 0.2, 0.0).unwrap(),
            None,
            Some(layers[6]),
        )
        .unwrap();
    check(&mut g, &d, "疎な層の上の調整");
    let clip_adj = d
        .add_adjustment_layer(
            "クリップの反転",
            AdjustmentSettings::invert(),
            None,
            Some(layers[7]),
        )
        .unwrap();
    d.set_layer_clipping(clip_adj, true).unwrap();
    check(&mut g, &d, "疎な下地へのクリッピングの調整");
    // 独立のグループ・通過のグループ・入れ子
    let g1 = d.group_layers(&[layers[1], layers[2]], "独立 1").unwrap();
    d.set_layer_blend_mode(g1, BlendMode::Normal).unwrap();
    d.set_layer_opacity(g1, 0.8, false).unwrap();
    let g2 = d
        .group_layers(&[layers[6], adj, layers[7]], "通過")
        .unwrap();
    d.set_layer_blend_mode(g2, BlendMode::PassThrough).unwrap();
    d.set_layer_opacity(g2, 0.6, false).unwrap();
    check(&mut g, &d, "疎な層のグループ");
    let g3 = d.group_layers(&[g1, layers[3]], "外").unwrap();
    d.set_layer_blend_mode(g3, BlendMode::Multiply).unwrap();
    check(&mut g, &d, "入れ子のグループ");
    // 描き足す（無かったタイルに画素ができる）・消す（描いたタイルを空にする）
    let l9 = layers[9];
    for (x, y) in [(3, 3), (70, 40), (120, 90), (17, 33)] {
        d.set_pixel(l9, x, y, Rgba8::new(255, 0, 255, 255)).unwrap();
        step(
            &mut g,
            &d,
            Channel::Color,
            &format!("({x}, {y}) に描き足す"),
        );
    }
    for &l in &layers[1..6] {
        d.set_pixel(l, 66, 10, Rgba8::new(0, 255, 0, 255)).unwrap();
    }
    step(&mut g, &d, Channel::Color, "複数の層の同じタイルへ描き足す");
    d.set_layer_visible(g1, false).unwrap();
    step(&mut g, &d, Channel::Color, "独立のグループを隠す");
    d.set_layer_visible(g1, true).unwrap();
    d.undo().unwrap();
    step(&mut g, &d, Channel::Color, "Undo");
    d.ungroup(g2).unwrap();
    step(&mut g, &d, Channel::Color, "通過のグループを解く");
}

// ───────── 独立して合成するグループ ─────────

#[test]
fn isolated_groups_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(101);
    let base = add_painted(&mut d, &mut rng, &[255]);
    let a = add_painted(&mut d, &mut rng, &[255, 170, 60]);
    let b = add_painted(&mut d, &mut rng, &[200, 255, 0]);
    d.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
    let group = d.group_layers(&[a, b], "組").unwrap();
    // 通過でない（Normal）なので中身を透明から合成する（中の Multiply は下地の base に掛からない）
    d.set_layer_blend_mode(group, BlendMode::Normal).unwrap();
    check(&mut g, &d, "独立のグループ（Normal・不透明度 1）");
    d.set_layer_opacity(group, 0.6, false).unwrap();
    check(&mut g, &d, "不透明度 0.6");
    for mode in [
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Hue,
        BlendMode::Luminosity,
        BlendMode::Divide,
    ] {
        d.set_layer_blend_mode(group, mode).unwrap();
        check(&mut g, &d, &format!("グループのモード {mode:?}"));
    }
    d.set_layer_blend_mode(group, BlendMode::Normal).unwrap();
    d.set_layer_opacity(group, 1.0, false).unwrap();
    // マスク
    d.add_layer_mask(group).unwrap();
    paint_mask(&mut d, group, &mut rng);
    check(&mut g, &d, "グループのマスク");
    d.set_layer_opacity(group, 0.7, false).unwrap();
    check(&mut g, &d, "マスクと不透明度");
    d.set_layer_mask_enabled(group, false).unwrap();
    check(&mut g, &d, "マスクを無効に");
    d.set_layer_mask_enabled(group, true).unwrap();
    d.set_layer_opacity(group, 1.0, false).unwrap();
    // 隠す・中の層を隠す
    d.set_layer_visible(group, false).unwrap();
    check(&mut g, &d, "グループを隠す");
    d.set_layer_visible(group, true).unwrap();
    d.set_layer_visible(b, false).unwrap();
    check(&mut g, &d, "中の層を隠す");
    d.set_layer_visible(b, true).unwrap();
    // グループが下地になるクリッピング
    let clip = add_painted(&mut d, &mut rng, &[255, 90]);
    d.move_layer_to(clip, None, 2).unwrap();
    d.set_layer_clipping(clip, true).unwrap();
    d.set_layer_blend_mode(clip, BlendMode::Screen).unwrap();
    check(&mut g, &d, "グループを下地にするクリッピング");
    // クリッピングされたグループ（独立して重ねる）
    let inner = add_painted(&mut d, &mut rng, &[255, 100, 255, 0]);
    let inner2 = add_painted(&mut d, &mut rng, &[180, 255]);
    d.set_layer_blend_mode(inner2, BlendMode::Overlay).unwrap();
    let clipped = d.group_layers(&[inner, inner2], "クリップの組").unwrap();
    d.move_layer_to(clipped, None, 1).unwrap();
    d.set_layer_clipping(clipped, true).unwrap();
    check(
        &mut g,
        &d,
        "クリッピングされたグループ（通過の指定でも透明から）",
    );
    d.set_layer_blend_mode(clipped, BlendMode::Normal).unwrap();
    d.set_layer_opacity(clipped, 0.5, false).unwrap();
    check(
        &mut g,
        &d,
        "クリッピングされたグループの Normal・不透明度 0.5",
    );
    let _ = base;
}

#[test]
fn nested_and_mixed_groups_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(103);
    let base = add_painted(&mut d, &mut rng, &[255]);
    // 独立のグループの中に、独立のグループ・通過のグループ（不透明度 0.5・マスク）・クリッピング・調整を持つ入れ子
    let l1 = add_painted(&mut d, &mut rng, &[255, 130]);
    let l2 = add_painted(&mut d, &mut rng, &[210, 255, 70]);
    d.set_layer_blend_mode(l2, BlendMode::Multiply).unwrap();
    let inner = d.group_layers(&[l1, l2], "内").unwrap();
    d.set_layer_blend_mode(inner, BlendMode::Normal).unwrap();
    d.set_layer_opacity(inner, 0.8, false).unwrap();
    let l3 = add_painted(&mut d, &mut rng, &[255, 0, 160]);
    d.set_layer_blend_mode(l3, BlendMode::Screen).unwrap();
    let pass_inner = add_painted(&mut d, &mut rng, &[240, 120]);
    d.set_layer_blend_mode(pass_inner, BlendMode::Overlay)
        .unwrap();
    let pass = d.group_layers(&[pass_inner], "通過").unwrap();
    d.set_layer_blend_mode(pass, BlendMode::PassThrough)
        .unwrap();
    d.set_layer_opacity(pass, 0.5, false).unwrap();
    let clip = add_painted(&mut d, &mut rng, &[255, 100]);
    d.set_layer_clipping(clip, true).unwrap();
    let outer = d.group_layers(&[inner, l3, pass, clip], "外").unwrap();
    d.set_layer_blend_mode(outer, BlendMode::Multiply).unwrap();
    check(&mut g, &d, "入れ子の独立のグループ");
    d.add_layer_mask(pass).unwrap();
    paint_mask(&mut d, pass, &mut rng);
    check(&mut g, &d, "通過のグループのマスク（フェードする）");
    d.set_layer_opacity(pass, 1.0, false).unwrap();
    check(&mut g, &d, "通過のグループのマスクだけ");
    let adj = d
        .add_adjustment_layer(
            "反転",
            AdjustmentSettings::hue_saturation(40.0, 0.3, -0.1).unwrap(),
            None,
            Some(l3),
        )
        .unwrap();
    check(
        &mut g,
        &d,
        "独立のグループの中の調整（中の合成だけを変える）",
    );
    d.set_layer_opacity(adj, 0.5, false).unwrap();
    check(&mut g, &d, "調整の不透明度");
    d.set_layer_opacity(outer, 0.6, false).unwrap();
    check(&mut g, &d, "外のグループの不透明度");
    // 構造の編集（グループの中へ・外へ・解く・Undo）
    d.move_layer_to(base, Some(inner), 0).unwrap();
    check(&mut g, &d, "層を独立のグループの中へ");
    d.ungroup(inner).unwrap();
    check(&mut g, &d, "グループを解く");
    d.undo().unwrap();
    check(&mut g, &d, "Undo で戻す");
    // 同じ形を 4 段入れ子にして、積みの上げ下げが合う
    let mut deep_layers = vec![add_painted(&mut d, &mut rng, &[255, 90])];
    for k in 0..4 {
        let l = add_painted(&mut d, &mut rng, &[255, 0, 200 - k * 30]);
        deep_layers.push(l);
        let grp = d.group_layers(&deep_layers, "段").unwrap();
        d.set_layer_blend_mode(
            grp,
            [BlendMode::Normal, BlendMode::PassThrough][k as usize % 2],
        )
        .unwrap();
        if k % 2 == 1 {
            d.set_layer_opacity(grp, 0.7, false).unwrap();
        }
        deep_layers = vec![grp];
    }
    check(&mut g, &d, "4 段の入れ子");
}

#[test]
fn deep_isolated_nesting_up_to_the_stack_still_matches_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(107);
    let mut inner = add_painted(&mut d, &mut rng, &[255, 120, 60]);
    let sibling = add_painted(&mut d, &mut rng, &[200, 255]);
    d.set_layer_blend_mode(sibling, BlendMode::Multiply)
        .unwrap();
    // 32 段（独立のグループは 2 語ずつ退避するので、積みの限り）
    for k in 0..32 {
        let grp = d.group_layers(&[inner], "段").unwrap();
        d.set_layer_blend_mode(grp, BlendMode::Normal).unwrap();
        d.set_layer_opacity(grp, if k % 3 == 0 { 0.9 } else { 1.0 }, false)
            .unwrap();
        inner = grp;
    }
    assert_eq!(supports(&d, Channel::Color), Ok(()));
    check(&mut g, &d, "32 段の独立のグループ");
}

// ───────── 調整の層 ─────────

fn bent_curve() -> Curve {
    Curve::new(vec![
        CurvePoint { x: 0., y: 0.15 },
        CurvePoint { x: 0.4, y: 0.7 },
        CurvePoint { x: 1., y: 0.85 },
    ])
    .unwrap()
}
fn inverted_curve() -> Curve {
    Curve::new(vec![
        CurvePoint { x: 0., y: 1. },
        CurvePoint { x: 1., y: 0. },
    ])
    .unwrap()
}
fn three_stop_ramp(alpha_end: f64) -> Ramp {
    Ramp::new(
        vec![
            ColorStop {
                position: 0.,
                color: Rgba8::new(20, 10, 90, 255),
                midpoint: 0.5,
            },
            ColorStop {
                position: 0.5,
                color: Rgba8::new(230, 60, 40, 255),
                midpoint: 0.5,
            },
            ColorStop {
                position: 1.,
                color: Rgba8::new(250, 240, 120, 255),
                midpoint: 0.5,
            },
        ],
        vec![
            OpacityStop {
                position: 0.,
                opacity: 1.,
                midpoint: 0.5,
            },
            OpacityStop {
                position: 1.,
                opacity: alpha_end,
                midpoint: 0.5,
            },
        ],
        None,
    )
    .unwrap()
}

/// 調整の全種類（名前つき）。成分ごとの表を引く種類と整数の式は CPU とバイトまで同じ、色相/彩度とカラーバランスは浮動小数。
fn adjustment_kinds() -> Vec<(&'static str, AdjustmentSettings)> {
    vec![
        ("反転", AdjustmentSettings::invert()),
        (
            "レベル補正",
            AdjustmentSettings::levels(0.2, 0.8, 1.7, 0.1, 0.9).unwrap(),
        ),
        (
            "色相/彩度",
            AdjustmentSettings::hue_saturation(70.0, 0.4, 0.15).unwrap(),
        ),
        (
            "色相/彩度（明度 −）",
            AdjustmentSettings::hue_saturation(-120.0, -0.5, -0.3).unwrap(),
        ),
        (
            "グラデーションマップ",
            AdjustmentSettings::gradient_map(GradientMap::new(three_stop_ramp(0.6), false)),
        ),
        (
            "グラデーションマップ（逆向き）",
            AdjustmentSettings::gradient_map(GradientMap::new(three_stop_ramp(1.0), true)),
        ),
        (
            "トーンカーブ",
            AdjustmentSettings::tone_curve(
                ToneCurves::identity()
                    .with_curve(ToneChannel::Composite, bent_curve())
                    .with_curve(ToneChannel::Red, inverted_curve())
                    .with_curve(ToneChannel::Blue, bent_curve()),
            ),
        ),
        (
            "カラーバランス",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [30.0, -10.0, 0.0],
                    [0.0, 40.0, -40.0],
                    [-20.0, 0.0, 60.0],
                    true,
                )
                .unwrap(),
            ),
        ),
        (
            "カラーバランス（輝度を保たない）",
            AdjustmentSettings::color_balance(
                ColorBalance::new(
                    [30.0, -10.0, 0.0],
                    [0.0, 40.0, -40.0],
                    [-20.0, 0.0, 60.0],
                    false,
                )
                .unwrap(),
            ),
        ),
        (
            "カラーバランス（何も動かさない）",
            AdjustmentSettings::color_balance(ColorBalance::neutral()),
        ),
        (
            "明るさ/コントラスト",
            AdjustmentSettings::brightness_contrast(BrightnessContrast::new(60.0, 30.0).unwrap()),
        ),
        (
            "2 値化",
            AdjustmentSettings::threshold(Threshold::new(100).unwrap()),
        ),
        (
            "ポスタリゼーション",
            AdjustmentSettings::posterize(Posterize::new(5).unwrap()),
        ),
        (
            "ポスタリゼーション（255 階調）",
            AdjustmentSettings::posterize(Posterize::new(255).unwrap()),
        ),
    ]
}

/// 調整の層 1 枚を、下の合成（透明・半透明・不透明の画素まじり）の上に、モード・不透明度・マスクを変えて置く。
#[test]
fn every_adjustment_kind_matches_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(211);
    let mut worst = 0u8;
    for (name, settings) in adjustment_kinds() {
        let mut d = doc();
        let base = add_painted(&mut d, &mut rng, &[255, 255, 170, 0, 90]);
        let over = add_painted(&mut d, &mut rng, &[200, 0, 255]);
        d.set_layer_blend_mode(over, BlendMode::Overlay).unwrap();
        let adj = d
            .add_adjustment_layer(name, settings.clone(), None, None)
            .unwrap();
        worst = worst.max(check(&mut g, &d, name));
        // 浮動小数の式（色相/彩度・カラーバランス）の 1 バイトの差（f32 と f64 の丸めの境）は、Divide のように下の値を上の値で割る
        // モードや、色相・彩度・カラー・輝度のように上の色の彩度から向きを決めるモードで増幅される（README の表示の許しの注意）ので、
        // それらのモードは調整の結果がバイトまで同じ種類だけで確かめる
        let exact = !matches!(
            settings.kind(),
            yolu_core::AdjustmentType::HueSaturation | yolu_core::AdjustmentType::ColorBalance
        );
        let modes: &[BlendMode] = if exact {
            &[
                BlendMode::Multiply,
                BlendMode::Hue,
                BlendMode::Screen,
                BlendMode::Divide,
            ]
        } else {
            &[BlendMode::Multiply, BlendMode::Screen, BlendMode::Overlay]
        };
        for &mode in modes {
            d.set_layer_blend_mode(adj, mode).unwrap();
            worst = worst.max(check(&mut g, &d, &format!("{name} {mode:?}")));
        }
        d.set_layer_blend_mode(adj, BlendMode::Normal).unwrap();
        d.set_layer_opacity(adj, 0.55, false).unwrap();
        worst = worst.max(check(&mut g, &d, &format!("{name} 不透明度 0.55")));
        d.add_layer_mask(adj).unwrap();
        paint_mask(&mut d, adj, &mut rng);
        worst = worst.max(check(&mut g, &d, &format!("{name} マスク")));
        // 調整の設定を差し替える（同じ層の命令・表が変わる）
        d.set_adjustment(adj, settings, false).unwrap();
        worst = worst.max(check(&mut g, &d, &format!("{name} 設定の再設定")));
        d.set_layer_visible(adj, false).unwrap();
        worst = worst.max(check(&mut g, &d, &format!("{name} 隠す")));
        let _ = base;
    }
    eprintln!("調整の全種類の最大差 {worst}");
}

#[test]
fn clipped_and_grouped_adjustments_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(223);
    let mut d = doc();
    let base = add_painted(&mut d, &mut rng, &[255]);
    let a = add_painted(&mut d, &mut rng, &[255, 170, 0]);
    // 下地 a に、クリッピングされた調整（下地の中の画素だけ変わる）と、クリッピングされた層
    let kinds = adjustment_kinds();
    let adj1 = d
        .add_adjustment_layer("クリップの調整", kinds[2].1.clone(), None, Some(a))
        .unwrap();
    d.set_layer_clipping(adj1, true).unwrap();
    check(&mut g, &d, "クリッピングされた調整");
    let c = add_painted(&mut d, &mut rng, &[255, 120]);
    d.set_layer_clipping(c, true).unwrap();
    d.set_layer_blend_mode(c, BlendMode::Multiply).unwrap();
    let adj2 = d
        .add_adjustment_layer("もう 1 つ", kinds[6].1.clone(), None, Some(c))
        .unwrap();
    d.set_layer_clipping(adj2, true).unwrap();
    d.set_layer_opacity(adj2, 0.5, false).unwrap();
    check(&mut g, &d, "クリッピングの調整を 2 つ・間に層");
    d.add_layer_mask(adj1).unwrap();
    paint_mask(&mut d, adj1, &mut rng);
    check(&mut g, &d, "クリッピングの調整のマスク");
    // 調整を下地にしたクリッピング（調整は下地にならないので、その上のクリッピングは描かない）
    let adj3 = d
        .add_adjustment_layer("下地にならない調整", kinds[0].1.clone(), None, None)
        .unwrap();
    let ghost = add_painted(&mut d, &mut rng, &[255]);
    d.set_layer_clipping(ghost, true).unwrap();
    let _ = adj3;
    check(&mut g, &d, "調整の上のクリッピングは描かない");
    // グループの中の調整は、グループの中の合成だけを変える
    let l1 = add_painted(&mut d, &mut rng, &[255, 100]);
    let inner_adj = d
        .add_adjustment_layer("中の調整", kinds[4].1.clone(), None, Some(l1))
        .unwrap();
    let grp = d.group_layers(&[l1, inner_adj], "調整入り").unwrap();
    d.set_layer_blend_mode(grp, BlendMode::Normal).unwrap();
    check(&mut g, &d, "独立のグループの中の調整");
    d.set_layer_blend_mode(grp, BlendMode::PassThrough).unwrap();
    check(&mut g, &d, "通過のグループの中の調整は下も変える");
    let _ = base;
}

#[test]
fn adjustments_on_scalar_channels_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(227);
    let mut d = doc();
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    for l in [a, b] {
        d.set_channel_enabled(l, Channel::Roughness, true).unwrap();
        paint_channel(&mut d, l, Channel::Roughness, &mut rng, &[255, 170, 0]);
    }
    d.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
    // スカラーのチャンネルに使える種類（色相/彩度・グラデーションマップ・カラーバランスは色のチャンネルだけ）。
    // トーンカーブはスカラーでは RGB 全体の曲線だけ
    for (name, settings) in adjustment_kinds() {
        if !settings.applies_to(yolu_core::ChannelKind::Scalar) {
            continue;
        }
        let adj = d
            .add_adjustment_layer(name, settings, Some(&[Channel::Roughness]), None)
            .unwrap();
        check_in(
            &mut g,
            &d,
            Channel::Roughness,
            TOLERANCE,
            &format!("Roughness {name}"),
        );
        d.remove_layer(adj).unwrap();
    }
}

// ───────── 法線の種類のチャンネル ─────────

fn normal_doc(rng: &mut Rng) -> (Document, LayerId, LayerId, LayerId) {
    let mut d = doc();
    let mut layers = Vec::new();
    for alphas in [&[255u8][..], &[255, 150, 0][..], &[200, 255, 80, 0][..]] {
        let l = d.add_layer("法線").unwrap();
        d.set_channel_enabled(l, Channel::Normal, true).unwrap();
        // 向きのある法線（単位ベクトルに近いバイト）と、正規化されていないバイトをまぜる
        for y in 0..d.height() {
            for x in 0..d.width() {
                let a = alphas[(x as usize + y as usize / 3) % alphas.len()];
                let c = if (x + y) % 5 == 0 {
                    Rgba8::new(rng.byte(), rng.byte(), rng.byte(), a)
                } else {
                    let (nx, ny) = (
                        (rng.byte() as f64 - 128.0) / 400.0,
                        (rng.byte() as f64 - 128.0) / 400.0,
                    );
                    let nz = (1.0 - nx * nx - ny * ny).sqrt();
                    let enc = |v: f64| ((v * 0.5 + 0.5) * 255.0).round() as u8;
                    Rgba8::new(enc(nx), enc(ny), enc(nz), a)
                };
                d.set_channel_pixel(l, Channel::Normal, x, y, c).unwrap();
            }
        }
        layers.push(l);
    }
    (d, layers[0], layers[1], layers[2])
}

#[test]
fn normal_channels_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(307);
    let (mut d, a, b, c) = normal_doc(&mut rng);
    assert_eq!(supports(&d, Channel::Normal), Ok(()));
    // 法線の重ね（通常は置き換え・Overlay は RNM）。法線の合成は単位ベクトルの式で、GPU は f32、CPU は f64。許しはほかの形と同じ（`TOLERANCE`）
    let normal_tolerance = TOLERANCE;
    let mut worst = check_in(&mut g, &d, Channel::Normal, normal_tolerance, "法線 通常");
    d.set_layer_blend_mode(b, BlendMode::Overlay).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 Overlay（RNM）",
    ));
    d.set_layer_opacity(b, 0.6, false).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 Overlay 不透明度",
    ));
    d.set_layer_blend_mode(c, BlendMode::Multiply).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 ほかのモードは置き換え",
    ));
    d.add_layer_mask(c).unwrap();
    paint_mask(&mut d, c, &mut rng);
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 マスク",
    ));
    // クリッピング・グループ・フェード
    d.set_layer_clipping(c, true).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 クリッピング",
    ));
    d.set_layer_blend_mode(c, BlendMode::Overlay).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 クリッピングの Overlay",
    ));
    let grp = d.group_layers(&[b, c], "組").unwrap();
    d.set_layer_blend_mode(grp, BlendMode::Normal).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 独立のグループ",
    ));
    d.set_layer_blend_mode(grp, BlendMode::PassThrough).unwrap();
    d.set_layer_opacity(grp, 0.5, false).unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 通過のグループのフェード",
    ));
    d.set_layer_opacity(grp, 1.0, false).unwrap();
    // 法線にも使える調整（反転・レベル補正）は色の式のまま
    for (name, settings) in adjustment_kinds().into_iter().take(2) {
        let adj = d
            .add_adjustment_layer(name, settings, Some(&[Channel::Normal]), None)
            .unwrap();
        worst = worst.max(check_in(
            &mut g,
            &d,
            Channel::Normal,
            normal_tolerance,
            &format!("法線 {name}"),
        ));
        d.remove_layer(adj).unwrap();
    }
    // 法線に使えない調整は描かない（CPU も描かない）
    let adj = d
        .add_adjustment_layer(
            "色相/彩度",
            adjustment_kinds()[2].1.clone(),
            Some(&[Channel::Color]),
            None,
        )
        .unwrap();
    worst = worst.max(check_in(
        &mut g,
        &d,
        Channel::Normal,
        normal_tolerance,
        "法線 色のチャンネルだけの調整",
    ));
    d.remove_layer(adj).unwrap();
    let _ = a;
    eprintln!("法線の最大差 {worst}");
}

// ───────── 効果のある文書 ─────────

fn blur_spec() -> FilterSpec {
    FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color])
}

#[test]
fn layers_with_filters_match_cpu_and_follow_edits_through_the_change_record() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(401);
    let mut d = Document::with_tile_size(96, 80, 16).unwrap();
    let base = add_painted(&mut d, &mut rng, &[255]);
    let a = d.add_layer("ぼかす層").unwrap();
    // 疎な絵（一部のタイルだけ）をぼかす: ぼかしは元のタイルの外へも広がる
    for y in 20..44 {
        for x in 30..62 {
            d.set_pixel(a, x, y, Rgba8::new(rng.byte(), rng.byte(), rng.byte(), 230))
                .unwrap();
        }
    }
    let filter = d.add_filter(a, FilterTarget::Content, blur_spec()).unwrap();
    assert_eq!(supports(&d, Channel::Color), Ok(()));
    let stats = step(&mut g, &d, Channel::Color, "ぼかしのある層");
    assert!(stats.cached_tiles > 0);
    // 1 画素を描くと、ぼかしの半径の分だけ広がった近くのタイルが更新される（変更の記録がその範囲を返す）
    d.set_pixel(a, 40, 30, Rgba8::new(255, 0, 0, 255)).unwrap();
    let stats = step(&mut g, &d, Channel::Color, "ぼかした層に描く");
    assert!(stats.updated_tiles >= 1);
    // タイルの境の近くに描く（隣のタイルの出力も変わる）
    d.set_pixel(a, 47, 31, Rgba8::new(0, 255, 0, 255)).unwrap();
    d.set_pixel(a, 48, 31, Rgba8::new(0, 255, 0, 255)).unwrap();
    step(&mut g, &d, Channel::Color, "ぼかしがタイルの境をまたぐ");
    // 強さ・無効・有効・外す
    d.set_filter_strength(a, filter, 0.4, false).unwrap();
    step(&mut g, &d, Channel::Color, "強さ 0.4");
    d.set_filter_enabled(a, filter, false).unwrap();
    step(&mut g, &d, Channel::Color, "フィルターを無効に");
    d.set_filter_enabled(a, filter, true).unwrap();
    step(&mut g, &d, Channel::Color, "有効に戻す");
    // 同じ層にマスクとモード・不透明度
    d.set_layer_blend_mode(a, BlendMode::Multiply).unwrap();
    d.set_layer_opacity(a, 0.8, false).unwrap();
    d.add_layer_mask(a).unwrap();
    paint_mask(&mut d, a, &mut rng);
    step(&mut g, &d, Channel::Color, "ぼかした層のマスクとモード");
    // 段を重ねる（シャープ・ノイズ・レベル補正）
    d.add_filter(
        a,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::sharpen(2, 1.5, 0)).channels(&[Channel::Color]),
    )
    .unwrap();
    d.add_filter(
        a,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::noise(0.4, 7, false)).channels(&[Channel::Color]),
    )
    .unwrap();
    step(&mut g, &d, Channel::Color, "段を重ねる");
    d.remove_filter(a, filter).unwrap();
    step(&mut g, &d, Channel::Color, "ぼかしを外す");
    let _ = base;
}

#[test]
fn global_filters_and_generators_and_mask_filters_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(409);
    let mut d = Document::with_tile_size(80, 64, 16).unwrap();
    let base = add_painted(&mut d, &mut rng, &[255]);
    let a = add_painted(&mut d, &mut rng, &[255, 160, 0]);
    // 正規化（全体の統計）・反転・レベル補正
    let normalize = d
        .add_filter(
            a,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::normalize()).channels(&[Channel::Color]),
        )
        .unwrap();
    step(&mut g, &d, Channel::Color, "正規化（全体の統計が要る段）");
    d.set_pixel(a, 5, 5, Rgba8::new(0, 0, 0, 255)).unwrap();
    step(
        &mut g,
        &d,
        Channel::Color,
        "正規化の層に描く（全体が変わる）",
    );
    d.remove_filter(a, normalize).unwrap();
    // マスクのフィルター
    d.add_layer_mask(a).unwrap();
    paint_mask(&mut d, a, &mut rng);
    let mask_filter = d
        .add_filter(
            a,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::blur(2)),
        )
        .unwrap();
    step(&mut g, &d, Channel::Color, "マスクのぼかし");
    d.set_mask_pixel(a, 20, 20, 255).unwrap();
    step(&mut g, &d, Channel::Color, "マスクに描く（ぼかしが広がる）");
    d.set_filter_enabled(a, mask_filter, false).unwrap();
    step(&mut g, &d, Channel::Color, "マスクのフィルターを無効に");
    d.set_filter_enabled(a, mask_filter, true).unwrap();
    // 反転のフィルターは何も無い所にも値を作る（マスクの外側がすべて隠す量 0 → 255）
    let invert_mask = d
        .add_filter(
            a,
            FilterTarget::Mask,
            FilterSpec::new(EffectSettings::invert()),
        )
        .unwrap();
    step(&mut g, &d, Channel::Color, "マスクの反転のフィルター");
    d.remove_filter(a, invert_mask).unwrap();
    // 塗りつぶしのグラデーション（全面）
    let fill = d
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(10, 20, 30, 255))],
            None,
        )
        .unwrap();
    let mut gradient = generator::Settings::new(generator::Kind::ShapeGradient);
    gradient.ramp = Some(three_stop_ramp(0.8));
    gradient.blend = generator::Blend::Replace;
    d.set_fill_gradient(fill, Channel::Color, Some(gradient), false)
        .unwrap();
    d.set_layer_blend_mode(fill, BlendMode::Overlay).unwrap();
    d.set_layer_opacity(fill, 0.7, false).unwrap();
    step(&mut g, &d, Channel::Color, "塗りつぶしのグラデーション");
    d.set_layer_opacity(fill, 0.3, false).unwrap();
    step(&mut g, &d, Channel::Color, "グラデーションの不透明度");
    d.set_fill_gradient(fill, Channel::Color, None, false)
        .unwrap();
    step(&mut g, &d, Channel::Color, "グラデーションを外す");
    // フィルターを掛けた塗りつぶし（全面。値が画素ごとに違う）
    let noisy = d
        .add_fill_layer(
            "ノイズの塗り",
            &[(Channel::Color, Rgba8::new(128, 120, 90, 200))],
            None,
        )
        .unwrap();
    let noise = d
        .add_filter(
            noisy,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::noise(0.8, 5, false)).channels(&[Channel::Color]),
        )
        .unwrap();
    d.set_layer_blend_mode(noisy, BlendMode::Overlay).unwrap();
    let stats = step(
        &mut g,
        &d,
        Channel::Color,
        "ノイズのフィルターを掛けた塗りつぶし",
    );
    assert!(stats.cached_tiles >= 5 * 4, "全面のタイルが常駐する");
    d.set_fill_value(
        noisy,
        Channel::Color,
        Some(Rgba8::new(10, 200, 90, 255)),
        false,
    )
    .unwrap();
    step(
        &mut g,
        &d,
        Channel::Color,
        "塗りの値を替える（出力が変わる）",
    );
    d.remove_filter(noisy, noise).unwrap();
    step(
        &mut g,
        &d,
        Channel::Color,
        "ノイズを外す（単色の塗りつぶしに戻る）",
    );
    let _ = base;
}

#[test]
fn effects_inside_isolated_groups_and_clipping_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut rng = Rng(419);
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    let base = add_painted(&mut d, &mut rng, &[255]);
    let a = add_painted(&mut d, &mut rng, &[255, 100, 0]);
    d.add_filter(a, FilterTarget::Content, blur_spec()).unwrap();
    let b = add_painted(&mut d, &mut rng, &[220, 255]);
    d.set_layer_clipping(b, true).unwrap();
    d.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
    d.add_filter(
        b,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(1)).channels(&[Channel::Color]),
    )
    .unwrap();
    let grp = d.group_layers(&[a, b], "効果入り").unwrap();
    d.set_layer_blend_mode(grp, BlendMode::Normal).unwrap();
    d.set_layer_opacity(grp, 0.75, false).unwrap();
    step(
        &mut g,
        &d,
        Channel::Color,
        "独立のグループの中の効果とクリッピング",
    );
    d.set_pixel(a, 30, 30, Rgba8::new(255, 255, 0, 255))
        .unwrap();
    step(&mut g, &d, Channel::Color, "効果のある層に描く");
    let _ = base;
}

#[test]
fn evaluated_layers_count_toward_the_budget_and_come_back_when_they_fit() {
    let Some(_) = gpu() else { return };
    let mut rng = Rng(431);
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    let base = add_painted(&mut d, &mut rng, &[255]);
    let limits = wgpu_limits();
    let options = ResidentOptions::default();
    let plain = resident_requirements(&d, Channel::Color, &options, &limits).unwrap();
    // ノイズのフィルターを掛けた塗りつぶし: 全部のタイルが常駐する（見積もりは全タイルぶん増える）
    let fill = d
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(128, 128, 128, 255))],
            None,
        )
        .unwrap();
    let noise = d
        .add_filter(
            fill,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::noise(0.8, 3, false)).channels(&[Channel::Color]),
        )
        .unwrap();
    let with_fill = resident_requirements(&d, Channel::Color, &options, &limits).unwrap();
    let tile = 16u64 * 16 * 4;
    let tiles = 4 * 4;
    assert_eq!(
        with_fill.tile_bytes - plain.tile_bytes,
        tiles * tile * 2,
        "全面の効果の層は全タイルを数える（GPU のコピーと同量の CPU のコピー）"
    );
    // 実際に常駐した量はこれ以下
    let mut g = gpu_with(options).unwrap();
    let stats = g.update(&d, Channel::Color).unwrap();
    assert!(stats.resident_bytes <= with_fill.total_bytes());
    assert_eq!(stats.cached_tiles as u64, 2 * tiles);
    // 見積もりが予算を超える（CPU へ落とす判断の材料）
    let small = ResidentOptions {
        resident_budget_bytes: with_fill.tile_bytes,
        ..options
    };
    let over = resident_requirements(&d, Channel::Color, &small, &limits).unwrap();
    assert!(over.total_bytes() > small.resident_budget_bytes, "{over:?}");
    // 効果を外すと、要る量は元に戻る（予算を超えて CPU へ落ちていても、収まれば GPU へ戻る）
    d.remove_filter(fill, noise).unwrap();
    let back = resident_requirements(&d, Channel::Color, &options, &limits).unwrap();
    assert_eq!(
        back.tile_bytes, plain.tile_bytes,
        "単色の塗りつぶしは面を持たない"
    );
    let _ = base;
}

fn wgpu_limits() -> wgpu::Limits {
    wgpu::Limits::default()
}

/// 追い出しの余白で動く小さな予算でも、効果のある文書の表示が CPU と同じ。
#[test]
fn evaluated_layers_under_eviction_keep_the_display_correct() {
    let mut rng = Rng(433);
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    let a = add_painted(&mut d, &mut rng, &[255, 120]);
    d.add_filter(a, FilterTarget::Content, blur_spec()).unwrap();
    let b = add_painted(&mut d, &mut rng, &[255, 60]);
    d.set_layer_opacity(b, 0.5, false).unwrap();
    // 表示 + 1 束 + 少しのタイル（全タイルが入らない）
    let tile = 16u64 * 16 * 4;
    let Some(mut g) = gpu_with(ResidentOptions {
        resident_budget_bytes: 64 * 64 * 4 + 2 * (tile * 2 * 4 + 400) + 2 * tile * 12,
        batch_tiles: 4,
        ..Default::default()
    }) else {
        return;
    };
    let stats = g.update(&d, Channel::Color).unwrap();
    assert!(stats.evicted_tiles > 0, "追い出しが起きる予算");
    let expected = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let actual = read(&mut g, d.bounds());
    assert!(max_diff(&expected, &actual) <= TOLERANCE);
}

// ───────── 予算が 1 束をちょうど保持できる境 ─────────

/// 面も命令も調整の表も多い文書（クリッピング・マスク・独立のグループ・調整）。
fn busy_doc(rng: &mut Rng) -> Document {
    let mut d = doc();
    let kinds = adjustment_kinds();
    let base = add_painted(&mut d, rng, &[255]);
    let a = add_painted(&mut d, rng, &[255, 170, 0]);
    d.add_layer_mask(a).unwrap();
    paint_mask(&mut d, a, rng);
    let adj = d
        .add_adjustment_layer("クリップの調整", kinds[4].1.clone(), None, Some(a))
        .unwrap();
    d.set_layer_clipping(adj, true).unwrap();
    let c = add_painted(&mut d, rng, &[255, 120]);
    d.set_layer_clipping(c, true).unwrap();
    d.set_layer_blend_mode(c, BlendMode::Multiply).unwrap();
    let l1 = add_painted(&mut d, rng, &[255, 100]);
    let inner = d
        .add_adjustment_layer("中の調整", kinds[1].1.clone(), None, Some(l1))
        .unwrap();
    let grp = d.group_layers(&[l1, inner], "組").unwrap();
    d.set_layer_blend_mode(grp, BlendMode::Normal).unwrap();
    let top = add_painted(&mut d, rng, &[255, 60]);
    d.set_layer_opacity(top, 0.6, false).unwrap();
    let _ = base;
    d
}

/// 常駐の予算が、表示と作業域と 1 束の全入力をちょうど保持できる量でも、束の途中で今の束のタイルを追い出さず（層が欠けない）、
/// 止まらず、CPU と同じ絵になる。保持できない予算は、止まらずに断る。
fn check_smallest_budget(d: &Document, label: &str) {
    let limits = wgpu_limits();
    let with = |budget: u64, batch_tiles: u32| ResidentOptions {
        resident_budget_bytes: budget,
        batch_tiles,
        ..Default::default()
    };
    let fits = |budget: u64| {
        resident_requirements(d, Channel::Color, &with(budget, 1), &limits)
            .unwrap()
            .fixed_bytes
            != u64::MAX
    };
    let (mut lo, mut hi) = (0u64, 1u64 << 22);
    assert!(fits(hi) && !fits(lo));
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let smallest = hi;
    let mut denied = gpu_with(with(smallest - 1, 1)).unwrap();
    let e = denied.update(d, Channel::Color).unwrap_err();
    assert!(e.to_string().contains("予算"), "{label}: {e}");
    // 境と、その少し上（束のタイル数が予算で決まる所）。追い出しは起きる（全タイルは入らない）
    for extra in [0u64, 1, 100, 3000, 12_000] {
        for batch_tiles in [1u32, 16] {
            let mut g = gpu_with(with(smallest + extra, batch_tiles)).unwrap();
            let what = format!("{label}・最小の予算 + {extra}・束 {batch_tiles}");
            check(&mut g, d, &what);
            let stats = g.stats();
            assert!(
                stats.resident_bytes <= smallest + extra,
                "{what}: 常駐 {} が予算を超える",
                stats.resident_bytes
            );
        }
    }
}

#[test]
fn display_is_correct_at_the_smallest_budget_that_holds_one_batch() {
    let Some(_) = gpu() else { return };
    let mut rng = Rng(467);
    check_smallest_budget(&busy_doc(&mut rng), "多い面・多い命令");
    // 面が 1 つ（束の全入力が 1 タイル分）の文書は、束の途中で今の束のタイルを追い出すと止まらなかった
    let mut single = doc();
    add_painted(&mut single, &mut rng, &[255, 90]);
    check_smallest_budget(&single, "1 面");
}
