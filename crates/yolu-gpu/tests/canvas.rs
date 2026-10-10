//! キャンバスの表示に使う常駐の合成を、アプリの文書の機能（マスク・塗りつぶし・チャンネルごとの合成・クリッピング・グループ・
//! 操作の列）で CPU の合成と照らす。GPU が扱えない文書は理由つきで断ることも確かめる。
//! 許しの範囲: レイヤーごとに半段切り上げで丸める式は同じで、GPU も CPU も f32 だが、シェーダーの演算の丸め（積和へのまとめなど）は
//! CPU の式（積和へまとめない）と同じとは限らないので、1 段ごとに最大 1 の差が出得る
//! （単独の段は `tests/parity.rs` が 1 以内を確かめる）。重なったレイヤーの差は次の段の式で増幅し（Overlay・HardLight で最大 2 倍、
//! ColorDodge・ColorBurn・VividLight・Divide はもっと大きい）、ここの多段の文書では 2 以内だった。下の `TOLERANCE` までを
//! 表示の許しとする。保存・書き出し・3D ビューの値は常に CPU の正本で、この差は載らない。
use yolu_core::{
    AdjustmentSettings, BlendMode, Channel, ChannelBlend, Document, LayerId, Rect, Rgba8, TileCoord,
};
use yolu_gpu::{
    resident_requirements, supports, GpuPainter, Options, ResidentCompositor, ResidentOptions,
    Unsupported, UpdateStats,
};

/// 表示の許し（1 画素の 1 バイトあたりの最大差）。多段の文書（マスク・塗りつぶし・グループ・チャンネルごとの合成）で 2 以内だった。
const TOLERANCE: u8 = 2;

#[path = "support/gpu_lease.rs"]
mod gpu_lease;
#[path = "support/require_gpu.rs"]
mod require_gpu;

fn gpu_with(options: ResidentOptions) -> Option<ResidentCompositor> {
    gpu_lease::lease();
    let g = match GpuPainter::new(Options::default()) {
        Ok(g) => g,
        Err(e) => {
            assert!(e.to_string().starts_with("GPU 利用不可:"), "{e}");
            require_gpu::skipped("キャンバスの GPU 試験", &e.to_string());
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
fn paint(d: &mut Document, layer: LayerId, rng: &mut Rng, alphas: &[u8]) {
    for y in 0..d.height() {
        for x in 0..d.width() {
            let a = alphas[(x as usize + y as usize / 3) % alphas.len()];
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
    check_channel(g, d, Channel::Color, what)
}
fn check_channel(g: &mut ResidentCompositor, d: &Document, channel: Channel, what: &str) -> u8 {
    g.update(d, channel).unwrap();
    let expected = d.composite_channel(channel, d.bounds()).unwrap();
    let actual = read(g, d.bounds());
    let max = max_diff(&expected, &actual);
    eprintln!("{what}: 最大差 {max}");
    assert!(max <= TOLERANCE, "{what}: 最大差 {max} > {TOLERANCE}");
    max
}

fn doc() -> Document {
    Document::with_tile_size(53, 37, 16).unwrap()
}

/// 入れ子 `levels` 段のグループ（一番内側に塗ったレイヤー 1 枚）。グループは mode で重ねる（通過でなければ独立して合成する）。
fn nested(d: &mut Document, levels: usize, mode: BlendMode) -> LayerId {
    let leaf = d.add_layer("葉").unwrap();
    let mut inner = leaf;
    for k in 0..levels {
        let g = d.group_layers(&[inner], &format!("組 {k}")).unwrap();
        d.set_layer_blend_mode(g, mode).unwrap();
        inner = g;
    }
    inner
}

#[test]
fn only_unknown_channels_and_too_deep_groups_are_refused_with_a_reason() {
    let mut d = doc();
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    assert_eq!(supports(&d, Channel::Color), Ok(()));
    // 法線の種類のチャンネル・調整レイヤー・独立して合成するグループは、GPU で合成できる
    assert_eq!(supports(&d, Channel::Normal), Ok(()));
    let fill = d
        .add_fill_layer("塗り", &[(Channel::Color, Rgba8::new(1, 2, 3, 200))], None)
        .unwrap();
    d.add_layer_mask(fill).unwrap();
    d.set_channel_blend(
        a,
        Channel::Color,
        ChannelBlend::new(Some(BlendMode::Screen), Some(0.5)),
        false,
    )
    .unwrap();
    let g = d.group_layers(&[a, b], "組").unwrap();
    d.add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.set_layer_blend_mode(g, BlendMode::Multiply).unwrap();
    d.set_layer_opacity(g, 0.5, false).unwrap();
    d.add_layer_mask(g).unwrap();
    d.set_mask_pixel(g, 3, 3, 255).unwrap();
    let top = d.add_layer("上").unwrap();
    d.move_layer_to(top, None, 1).unwrap(); // グループのすぐ上
    d.set_layer_clipping(top, true).unwrap(); // グループが下地になる
    assert_eq!(supports(&d, Channel::Color), Ok(()));
    assert_eq!(supports(&d, Channel::Normal), Ok(()));
    // 文書にないチャンネル
    let missing = Channel::from_index(40).unwrap();
    assert_eq!(supports(&d, missing), Err(Unsupported::UnknownChannel));
    assert_eq!(
        supports(&d, missing).unwrap_err().reason(),
        "プロジェクトにないチャンネル"
    );
    // グループの入れ子: 独立して合成するグループは 2 語ずつ退避するので 32 段まで
    let mut deep = doc();
    nested(&mut deep, 32, BlendMode::Multiply);
    assert_eq!(supports(&deep, Channel::Color), Ok(()), "32 段");
    let mut too_deep = doc();
    nested(&mut too_deep, 33, BlendMode::Multiply);
    assert_eq!(
        supports(&too_deep, Channel::Color),
        Err(Unsupported::GroupDepth)
    );
    assert_eq!(
        supports(&too_deep, Channel::Color).unwrap_err().reason(),
        "グループの入れ子が GPU で合成できる深さを超える"
    );
    // 通過のグループは中身をそのまま下へ重ねる（平らにする）ので、深くしても積みを使わない。不透明度が 1 でなければ 1 語ずつ使う
    let mut passes = doc();
    nested(&mut passes, 40, BlendMode::PassThrough);
    assert_eq!(supports(&passes, Channel::Color), Ok(()), "平らになる通過");
    // 見えないグループは描かないので、深さに数えない
    let outer = too_deep
        .layers()
        .iter()
        .find(|l| l.is_group() && l.parent().is_none())
        .unwrap()
        .id();
    too_deep.set_layer_visible(outer, false).unwrap();
    assert_eq!(
        supports(&too_deep, Channel::Color),
        Ok(()),
        "見えないグループ"
    );
}

/// 通過のグループのすぐ上に並ぶクリッピングのレイヤーが何も描かない（隠す・不透明度 0・空のグループ）なら、core の計画は組を持たない
/// 通過のグループのまま。描くクリッピングのレイヤーがあるときだけ、独立して合成するグループになる。どちらも CPU と同じ画素になる。
#[test]
fn clipping_layers_on_a_pass_through_group_follow_the_cpu_plan() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(61);
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    paint(&mut d, a, &mut rng, &[255, 120]);
    paint(&mut d, b, &mut rng, &[200, 255, 0]);
    d.set_layer_blend_mode(b, BlendMode::Multiply).unwrap();
    let group = d.group_layers(&[a, b], "組").unwrap();
    check(&mut g, &d, "通過のグループ");
    let top = d.add_layer("上").unwrap();
    paint(&mut d, top, &mut rng, &[255, 90]);
    d.set_layer_clipping(top, true).unwrap();
    check(
        &mut g,
        &d,
        "描くクリッピングのレイヤーが下地のグループを独立にする",
    );
    d.set_layer_visible(top, false).unwrap();
    check(&mut g, &d, "隠したクリッピングのレイヤー");
    d.set_layer_visible(top, true).unwrap();
    d.set_layer_opacity(top, 0.0, false).unwrap();
    check(&mut g, &d, "不透明度 0");
    d.set_layer_opacity(top, 1.0, false).unwrap();
    check(&mut g, &d, "戻した");
    d.remove_layer(top).unwrap();
    // 空のグループのクリッピングは描かない
    let empty = d.add_group("空", None).unwrap();
    d.set_layer_clipping(empty, true).unwrap();
    check(&mut g, &d, "空のグループのクリッピング");
    // 描く調整レイヤーのクリッピングは組に入る（グループは独立になる）。隠せば入らない
    let adj = d
        .add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
        .unwrap();
    d.move_layer_to(adj, None, 1).unwrap();
    d.set_layer_clipping(adj, true).unwrap();
    check(&mut g, &d, "調整のクリッピング");
    d.set_layer_visible(adj, false).unwrap();
    check(&mut g, &d, "隠した調整のクリッピング");
    let hidden_clip = d.add_layer("隠したクリッピング").unwrap();
    d.move_layer_to(hidden_clip, None, 1).unwrap();
    paint(&mut d, hidden_clip, &mut rng, &[255]);
    d.set_layer_clipping(hidden_clip, true).unwrap();
    d.set_layer_visible(hidden_clip, false).unwrap();
    check(&mut g, &d, "隠したクリッピングを上に持つ通過のグループ");
    let _ = (group, empty);
}

#[test]
fn unsupported_update_is_refused_without_breaking_the_compositor() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let a = d.add_layer("a").unwrap();
    let mut rng = Rng(7);
    paint(&mut d, a, &mut rng, &[255, 120]);
    check(&mut g, &d, "入れ子の前");
    let deep = nested(&mut d, 33, BlendMode::Multiply);
    let e = g.update(&d, Channel::Color).unwrap_err();
    assert!(e.to_string().contains("入れ子"), "{e}");
    // GPU は失敗扱いにならず、扱える文書に戻れば続けて使える
    d.remove_layer(deep).unwrap();
    check(&mut g, &d, "入れ子を外したあと");
    let e = g.update(&d, Channel::from_index(40).unwrap()).unwrap_err();
    assert!(e.to_string().contains("チャンネル"), "{e}");
    check(&mut g, &d, "文書にないチャンネルを断ったあと");
}

#[test]
fn masks_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(11);
    let base = d.add_layer("下").unwrap();
    paint(&mut d, base, &mut rng, &[255]);
    let top = d.add_layer("上").unwrap();
    paint(&mut d, top, &mut rng, &[255, 200, 90, 255]);
    d.add_layer_mask(top).unwrap();
    paint_mask(&mut d, top, &mut rng);
    check(&mut g, &d, "マスク");
    d.set_layer_mask_inverted(top, true).unwrap();
    check(&mut g, &d, "マスク 反転");
    d.set_layer_mask_density(top, 0.4, false).unwrap();
    check(&mut g, &d, "マスク 反転 濃度 0.4");
    d.set_layer_mask_inverted(top, false).unwrap();
    check(&mut g, &d, "マスク 濃度 0.4");
    d.set_layer_opacity(top, 0.65, false).unwrap();
    check(&mut g, &d, "マスク 濃度 0.4 不透明度 0.65");
    d.set_layer_mask_enabled(top, false).unwrap();
    check(&mut g, &d, "マスク 無効");
    d.set_layer_mask_enabled(top, true).unwrap();
    for mode in [
        BlendMode::Multiply,
        BlendMode::Overlay,
        BlendMode::Hue,
        BlendMode::Difference,
    ] {
        d.set_layer_blend_mode(top, mode).unwrap();
        check(&mut g, &d, &format!("マスク {mode:?}"));
    }
    // 下地のマスクと、クリッピングされたレイヤーのマスク
    d.set_layer_blend_mode(top, BlendMode::Normal).unwrap();
    d.add_layer_mask(base).unwrap();
    paint_mask(&mut d, base, &mut rng);
    d.set_layer_clipping(top, true).unwrap();
    check(&mut g, &d, "下地のマスクとクリップのマスク");
    d.set_layer_mask_density(base, 0.7, false).unwrap();
    check(&mut g, &d, "下地のマスクの濃度");
    d.remove_layer_mask(top).unwrap();
    check(&mut g, &d, "マスクを外す");
    d.undo().unwrap();
    check(&mut g, &d, "外したマスクを戻す");
    // マスクのストロークで 1 画素を隠す
    d.set_mask_pixel(top, 5, 5, 255).unwrap();
    check(&mut g, &d, "マスクの 1 画素");
}

#[test]
fn fill_layers_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(13);
    let base = d.add_layer("下").unwrap();
    paint(&mut d, base, &mut rng, &[255, 255, 100]);
    let fill = d
        .add_fill_layer(
            "塗り",
            &[
                (Channel::Color, Rgba8::new(200, 40, 90, 180)),
                (Channel::Roughness, Rgba8::new(120, 120, 120, 255)),
            ],
            None,
        )
        .unwrap();
    check(&mut g, &d, "塗りつぶし");
    check_channel(&mut g, &d, Channel::Roughness, "塗りつぶし Roughness");
    d.set_fill_value(
        fill,
        Channel::Color,
        Some(Rgba8::new(10, 220, 30, 255)),
        false,
    )
    .unwrap();
    check(&mut g, &d, "塗りつぶしの値を替える");
    d.add_layer_mask(fill).unwrap();
    paint_mask(&mut d, fill, &mut rng);
    check(&mut g, &d, "塗りつぶし + マスク");
    for mode in [
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::SoftLight,
        BlendMode::Color,
    ] {
        d.set_layer_blend_mode(fill, mode).unwrap();
        check(&mut g, &d, &format!("塗りつぶし {mode:?}"));
    }
    // 塗りつぶしの上のクリッピング、塗りつぶしを下地にする
    d.set_layer_blend_mode(fill, BlendMode::Normal).unwrap();
    let clip = d.add_layer("クリップ").unwrap();
    paint(&mut d, clip, &mut rng, &[255, 60]);
    d.set_layer_clipping(clip, true).unwrap();
    check(&mut g, &d, "塗りつぶしの下地へのクリップ");
    d.set_fill_value(fill, Channel::Color, None, false).unwrap();
    check(
        &mut g,
        &d,
        "値を消した塗りつぶし（描かず、クリップも落ちる）",
    );
    d.undo().unwrap();
    check(&mut g, &d, "値を戻す");
    d.set_layer_visible(fill, false).unwrap();
    check(&mut g, &d, "塗りつぶしを隠す");
}

#[test]
fn per_channel_blend_matches_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(17);
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    paint(&mut d, a, &mut rng, &[255]);
    paint(&mut d, b, &mut rng, &[255, 140]);
    d.set_channel_blend(
        b,
        Channel::Color,
        ChannelBlend::new(Some(BlendMode::Multiply), Some(0.5)),
        false,
    )
    .unwrap();
    check(&mut g, &d, "チャンネルごとの合成（モードと不透明度）");
    d.set_channel_opacity(b, Channel::Color, Some(0.0), false)
        .unwrap();
    check(&mut g, &d, "チャンネルの不透明度 0 は描かない");
    d.set_channel_blend(b, Channel::Color, ChannelBlend::default(), false)
        .unwrap();
    d.set_layer_opacity(b, 0.0, false).unwrap();
    d.set_channel_opacity(b, Channel::Color, Some(0.8), false)
        .unwrap();
    check(
        &mut g,
        &d,
        "レイヤーの不透明度 0 でもチャンネルの不透明度が勝つ",
    );
    // ほかのチャンネルはレイヤーの設定のまま
    d.set_channel_enabled(b, Channel::Roughness, true).unwrap();
    d.set_pixel(b, 3, 3, Rgba8::new(1, 1, 1, 255)).unwrap();
    check_channel(&mut g, &d, Channel::Roughness, "Roughness はレイヤーの設定");
}

/// 平らにする: 通過で不透明度 1 のグループは、中身を下へそのまま重ねるのと同じ。
#[test]
fn pass_through_groups_are_flattened_and_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(19);
    let ids: Vec<LayerId> = (0..5)
        .map(|k| {
            let l = d.add_layer("レイヤー").unwrap();
            paint(&mut d, l, &mut rng, &[255, 170, 60, 255][k % 4..]);
            l
        })
        .collect();
    d.set_layer_blend_mode(ids[2], BlendMode::Multiply).unwrap();
    d.set_layer_blend_mode(ids[3], BlendMode::Screen).unwrap();
    d.set_layer_opacity(ids[3], 0.6, false).unwrap();
    check(&mut g, &d, "グループの前");
    let group = d.group_layers(&[ids[1], ids[2], ids[3]], "組").unwrap();
    check(&mut g, &d, "通過のグループ");
    // グループの中のクリッピング（グループの一番下のレイヤーは、印があっても下地）
    d.set_layer_clipping(ids[2], true).unwrap();
    d.set_layer_clipping(ids[1], true).unwrap();
    check(&mut g, &d, "グループの中のクリッピング");
    d.set_layer_visible(group, false).unwrap();
    check(&mut g, &d, "グループを隠す");
    d.set_layer_visible(group, true).unwrap();
    d.set_layer_visible(ids[2], false).unwrap();
    check(&mut g, &d, "グループの中のレイヤーを隠す");
    d.set_layer_visible(ids[2], true).unwrap();
    // 入れ子と、グループの外のレイヤーをグループの中へ・外へ
    let outer = d.group_layers(&[group, ids[4]], "外").unwrap();
    check(&mut g, &d, "入れ子のグループ");
    d.move_layer_to(ids[0], Some(group), 1).unwrap();
    check(&mut g, &d, "レイヤーをグループの中へ");
    d.move_layer_to(ids[1], None, 0).unwrap();
    check(&mut g, &d, "レイヤーをグループの外へ");
    // グループの下地にクリッピングのレイヤーを重ねない通常のグループの上に、クリッピングされた通常のレイヤー
    d.ungroup(group).unwrap();
    check(&mut g, &d, "グループを解く");
    d.undo().unwrap();
    check(&mut g, &d, "解いたグループを戻す");
    d.set_layer_opacity(outer, 0.5, false).unwrap();
    check(
        &mut g,
        &d,
        "不透明度 0.5 の通過のグループ（下とフェードする）",
    );
    d.set_layer_opacity(outer, 1.0, false).unwrap();
    check(&mut g, &d, "平らな通過に戻した");
}

#[test]
fn clipping_stacks_and_hidden_bases_match_cpu() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(23);
    let mut ids = Vec::new();
    for k in 0..7 {
        let l = d.add_layer("レイヤー").unwrap();
        paint(&mut d, l, &mut rng, &[255, 130, 40, 200]);
        d.set_layer_clipping(l, k % 3 != 0).unwrap();
        d.set_layer_opacity(l, 0.45 + 0.08 * k as f64, false)
            .unwrap();
        d.set_layer_blend_mode(l, BlendMode::LAYER_MODES[(k * 5) % 26])
            .unwrap();
        ids.push(l);
    }
    check(&mut g, &d, "クリッピングの組");
    for &l in &ids {
        d.set_layer_visible(l, false).unwrap();
        check(&mut g, &d, "1 枚隠す");
        d.set_layer_visible(l, true).unwrap();
    }
    check(&mut g, &d, "全部戻す");
}

/// 変化の記録だけで更新する（途中の差分）。レイヤーの追加・削除・並べ替え・入れ子・Undo/Redo・プロパティ・マスク・塗りつぶしの操作の
/// どれのあとでも、全面の合成と同じになる。
#[test]
fn incremental_updates_follow_every_kind_of_edit() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(29);
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    paint(&mut d, a, &mut rng, &[255]);
    paint(&mut d, b, &mut rng, &[255, 100]);
    check(&mut g, &d, "初め");
    let c = d.add_layer("c").unwrap();
    check(&mut g, &d, "空のレイヤーを足す");
    paint(&mut d, c, &mut rng, &[200, 0, 255]);
    check(&mut g, &d, "足したレイヤーに描く");
    d.move_layer(c, 0).unwrap();
    check(&mut g, &d, "並べ替え");
    let group = d.group_layers(&[a, b], "組").unwrap();
    check(&mut g, &d, "グループにする");
    d.set_layer_visible(group, false).unwrap();
    check(&mut g, &d, "グループを隠す");
    d.undo().unwrap();
    check(&mut g, &d, "取消");
    d.redo().unwrap();
    check(&mut g, &d, "やり直し");
    d.set_layer_visible(group, true).unwrap();
    d.move_layer_to(c, Some(group), 0).unwrap();
    check(&mut g, &d, "グループへ移す");
    d.ungroup(group).unwrap();
    check(&mut g, &d, "グループを解く");
    d.add_layer_mask(b).unwrap();
    paint_mask(&mut d, b, &mut rng);
    check(&mut g, &d, "マスクを足して塗る");
    d.set_layer_mask_density(b, 0.3, false).unwrap();
    check(&mut g, &d, "マスクの濃度");
    d.set_layer_mask_inverted(b, true).unwrap();
    check(&mut g, &d, "マスクの反転");
    d.set_layer_mask_enabled(b, false).unwrap();
    check(&mut g, &d, "マスクを無効に");
    d.undo().unwrap();
    check(&mut g, &d, "取消");
    let fill = d
        .add_fill_layer(
            "塗り",
            &[(Channel::Color, Rgba8::new(9, 99, 199, 160))],
            None,
        )
        .unwrap();
    check(&mut g, &d, "塗りつぶしを足す");
    d.set_fill_value(
        fill,
        Channel::Color,
        Some(Rgba8::new(199, 99, 9, 255)),
        false,
    )
    .unwrap();
    check(&mut g, &d, "塗りつぶしの値");
    d.set_layer_blend_mode(fill, BlendMode::Overlay).unwrap();
    check(&mut g, &d, "塗りつぶしのモード");
    d.set_channel_opacity(fill, Channel::Color, Some(0.3), false)
        .unwrap();
    check(&mut g, &d, "チャンネルの不透明度");
    d.duplicate_layer(b, None).unwrap();
    check(&mut g, &d, "複製");
    d.remove_layer(c).unwrap();
    check(&mut g, &d, "削除");
    d.undo().unwrap();
    check(&mut g, &d, "削除の取消");
    d.set_layer_clipping(b, true).unwrap();
    check(&mut g, &d, "クリッピング");
    // ストロークの途中と取消
    let brush = yolu_core::BrushSettings {
        radius: 5.0,
        hardness: 0.8,
        color: Rgba8::new(250, 30, 30, 255),
        ..Default::default()
    };
    let mut s = d.begin_stroke(a, &brush).unwrap();
    s.add_point(&mut d, 20.0, 20.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    check(&mut g, &d, "ストロークの途中");
    s.add_point(&mut d, 40.0, 25.0, 1.0, yolu_core::glam::DVec2::ZERO)
        .unwrap();
    check(&mut g, &d, "ストロークの続き");
    d.cancel_stroke(s);
    check(&mut g, &d, "ストロークの取消");
}

/// 更新して全面の合成と照らし、そのときの記録を返す。
fn step(g: &mut ResidentCompositor, d: &Document, what: &str) -> UpdateStats {
    check(g, d, what);
    g.stats()
}

/// レイヤーの追加・削除・並べ替え・表示・不透明度・入れ子は、全面を合成し直さず、core の変更記録が名指す（そのレイヤーの持つ）タイルだけを
/// 合成し直す。全面の再合成のままでも画素は合うので、`updated_tiles` で確かめる。
#[test]
fn structural_edits_update_only_the_tiles_the_change_record_names() {
    let Some(mut g) = gpu() else { return };
    // 64² を 16² のタイル 16 枚で。a は全面、b は 3 タイル（(0,0)・(1,0)・(2,1)）、c は後から 1 タイル（(3,3)）
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    let mut rng = Rng(53);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, &mut rng, &[255, 140]);
    let b = d.add_layer("b").unwrap();
    for (x, y) in [(1, 1), (20, 1), (40, 20)] {
        d.set_pixel(b, x, y, Rgba8::new(200, 40, 90, 220)).unwrap();
    }
    let first = step(&mut g, &d, "初め");
    assert_eq!(first.updated_tiles, 16, "初めは全面");
    let same = g.update(&d, Channel::Color).unwrap();
    assert_eq!((same.updated_tiles, same.uploaded_tiles), (0, 0));
    let c = d.add_layer("c").unwrap();
    let s = step(&mut g, &d, "空のレイヤーを足す");
    assert_eq!(s.updated_tiles, 0, "空のレイヤーは何も変えない: {s:?}");
    d.set_pixel(c, 60, 60, Rgba8::new(10, 220, 30, 255))
        .unwrap();
    let s = step(&mut g, &d, "足したレイヤーに 1 画素");
    assert_eq!(s.updated_tiles, 1, "{s:?}");
    d.move_layer(b, 2).unwrap();
    let s = step(&mut g, &d, "並べ替え");
    assert_eq!(s.updated_tiles, 3, "b の持つタイルだけ: {s:?}");
    assert_eq!(s.uploaded_tiles, 0, "常駐のタイルは上げ直さない: {s:?}");
    d.set_layer_opacity(b, 0.5, false).unwrap();
    let s = step(&mut g, &d, "不透明度");
    assert_eq!(s.updated_tiles, 3, "{s:?}");
    d.set_layer_visible(b, false).unwrap();
    let s = step(&mut g, &d, "隠す");
    assert_eq!(s.updated_tiles, 3, "{s:?}");
    d.set_layer_visible(b, true).unwrap();
    let s = step(&mut g, &d, "見せる");
    assert_eq!(s.updated_tiles, 3, "{s:?}");
    let before_remove = g.stats().cached_tiles;
    d.remove_layer(b).unwrap();
    let s = step(&mut g, &d, "削除");
    assert_eq!(s.updated_tiles, 3, "{s:?}");
    assert_eq!(
        s.cached_tiles,
        before_remove - 3,
        "消えたレイヤーの常駐を手放す"
    );
    d.undo().unwrap();
    let s = step(&mut g, &d, "削除の取消");
    assert_eq!(s.updated_tiles, 3, "{s:?}");
    d.group_layers(&[b, c], "組").unwrap();
    let s = step(&mut g, &d, "グループにする");
    assert_eq!(s.updated_tiles, 4, "中身（b の 3 と c の 1）だけ: {s:?}");
    // 塗りつぶしはキャンバス全体が変わる
    d.add_fill_layer(
        "塗り",
        &[(Channel::Color, Rgba8::new(9, 99, 199, 90))],
        None,
    )
    .unwrap();
    let s = step(&mut g, &d, "塗りつぶしを足す");
    assert_eq!(s.updated_tiles, 16, "{s:?}");
}

/// 隠したレイヤー・不透明度 0・外したマスクなど、計画から外れた面のタイルは常駐から手放す（GPU バッファと同量の CPU のコピーが
/// 予算に数えられ続けて、見えるレイヤーのタイルを追い出さないように）。常駐は見積もりへ戻り、戻したときは変更記録のタイルを上げ直す。
#[test]
fn faces_that_leave_the_plan_are_released_and_come_back_with_the_change_record() {
    let Some(mut g) = gpu() else { return };
    let limits = wgpu::Limits::default();
    let options = ResidentOptions::default();
    let mut d = Document::with_tile_size(64, 64, 16).unwrap();
    let mut rng = Rng(59);
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    let c = d.add_layer("c").unwrap();
    for l in [a, b, c] {
        paint(&mut d, l, &mut rng, &[255, 180, 90]);
    }
    d.add_layer_mask(b).unwrap();
    paint_mask(&mut d, b, &mut rng);
    let estimate = |d: &Document| {
        resident_requirements(d, Channel::Color, &options, &limits)
            .unwrap()
            .total_bytes()
    };
    // a・b・b のマスク・c で 16 タイルずつ
    let s = step(&mut g, &d, "全部見える");
    assert_eq!((s.cached_tiles, s.resident_bytes), (64, estimate(&d)));
    // 隠したレイヤー（面とマスクの両方）
    d.set_layer_visible(b, false).unwrap();
    let s = step(&mut g, &d, "b を隠す");
    assert_eq!(s.cached_tiles, 32, "b の面とマスクを手放す: {s:?}");
    assert_eq!(s.resident_bytes, estimate(&d), "見積もりへ戻る");
    d.set_layer_visible(b, true).unwrap();
    let s = step(&mut g, &d, "b を見せる");
    assert_eq!(s.cached_tiles, 64);
    assert_eq!(s.uploaded_tiles, 32, "手放した面は上げ直す: {s:?}");
    assert_eq!(s.resident_bytes, estimate(&d));
    // 不透明度 0
    d.set_layer_opacity(c, 0.0, false).unwrap();
    let s = step(&mut g, &d, "c の不透明度 0");
    assert_eq!(s.cached_tiles, 48, "{s:?}");
    assert_eq!(s.resident_bytes, estimate(&d));
    d.set_layer_opacity(c, 1.0, false).unwrap();
    let s = step(&mut g, &d, "c の不透明度を戻す");
    assert_eq!((s.cached_tiles, s.uploaded_tiles), (64, 16), "{s:?}");
    // 外した（無効にした）マスク
    d.set_layer_mask_enabled(b, false).unwrap();
    let s = step(&mut g, &d, "b のマスクを無効に");
    assert_eq!(s.cached_tiles, 48, "マスクの面だけ手放す: {s:?}");
    assert_eq!(s.resident_bytes, estimate(&d));
    d.set_layer_mask_enabled(b, true).unwrap();
    let s = step(&mut g, &d, "b のマスクを有効に");
    assert_eq!((s.cached_tiles, s.uploaded_tiles), (64, 16), "{s:?}");
    // 隠したグループの中のレイヤー
    let group = d.group_layers(&[a, b], "組").unwrap();
    d.set_layer_visible(group, false).unwrap();
    let s = step(&mut g, &d, "グループごと隠す");
    assert_eq!(s.cached_tiles, 16, "c だけ: {s:?}");
    assert_eq!(s.resident_bytes, estimate(&d));
    d.set_layer_visible(group, true).unwrap();
    let s = step(&mut g, &d, "グループを見せる");
    assert_eq!((s.cached_tiles, s.resident_bytes), (64, estimate(&d)));
}

#[test]
fn absent_tiles_are_not_resident_and_cleared_when_they_disappear() {
    let Some(mut g) = gpu() else { return };
    let mut d = Document::with_tile_size(4096, 4096, 128).unwrap();
    let l = d.add_layer("疎").unwrap();
    d.set_pixel(l, 130, 5, Rgba8::new(10, 20, 30, 255)).unwrap();
    d.set_pixel(l, 4000, 4000, Rgba8::new(40, 50, 60, 255))
        .unwrap();
    let stats = g.update(&d, Channel::Color).unwrap();
    assert_eq!(stats.updated_tiles, 1024);
    assert_eq!(stats.uploaded_tiles, 2, "描いた 2 タイルだけを常駐させる");
    assert_eq!(stats.cached_tiles, 2);
    // 常駐は表示のテクスチャ（4096² × 4 = 64 MiB）と作業域、描いた 2 タイル。全タイルを常駐させると 128 MiB を超える。
    assert!(
        stats.resident_bytes < (64 << 20) + (4 << 20),
        "{}",
        stats.resident_bytes
    );
    let rect = Rect::new(128, 0, 128, 128);
    let got = read(&mut g, rect);
    assert_eq!(&got[(5 * 128 + 2) * 4..][..4], &[10, 20, 30, 255]);
    // タイルを空にすると、常駐も表示も消える
    let zero = vec![0u8; 128 * 128 * 4];
    d.import_tile(l, Channel::Color, TileCoord::new(1, 0), &zero)
        .unwrap();
    let after = g.update(&d, Channel::Color).unwrap();
    assert_eq!(after.updated_tiles, 1);
    assert_eq!(after.cached_tiles, 1, "空になったタイルの常駐を捨てる");
    assert!(read(&mut g, rect).iter().all(|&b| b == 0));
    assert_eq!(
        &read(&mut g, Rect::new(4000, 4000, 1, 1)),
        &[40, 50, 60, 255]
    );
}

/// 乗算済みの表示: egui の `Color32::from_rgba_unmultiplied` と同じ式で、straight の表示から CPU で変換した値と同じ。
#[test]
fn premultiplied_display_is_the_cpu_conversion_of_the_straight_display() {
    let Some(mut straight) = gpu() else { return };
    let Some(mut premultiplied) = gpu_with(ResidentOptions {
        premultiplied_display: true,
        ..Default::default()
    }) else {
        return;
    };
    let mut d = doc();
    let mut rng = Rng(31);
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    paint(&mut d, a, &mut rng, &[255, 0, 1, 2, 127, 128, 254, 77, 200]);
    paint(&mut d, b, &mut rng, &[0, 255, 3, 90, 253, 129]);
    d.set_layer_opacity(b, 0.6, false).unwrap();
    straight.update(&d, Channel::Color).unwrap();
    premultiplied.update(&d, Channel::Color).unwrap();
    let s = read(&mut straight, d.bounds());
    let p = read(&mut premultiplied, d.bounds());
    // egui の ecolor 0.36 の mul_frac_round と同じ
    let mul = |v: u8, a: u8| -> u8 {
        let p = u16::from(v) * u16::from(a) + 128;
        ((p + (p >> 8)) >> 8) as u8
    };
    let mut seen_partial = false;
    for (s, p) in s.as_chunks::<4>().0.iter().zip(p.as_chunks::<4>().0) {
        let expected = match s[3] {
            0 => [0, 0, 0, 0],
            255 => [s[0], s[1], s[2], 255],
            a => {
                seen_partial = true;
                [mul(s[0], a), mul(s[1], a), mul(s[2], a), a]
            }
        };
        assert_eq!(*p, expected, "straight = {s:?}");
    }
    assert!(seen_partial);
    // CPU の合成との差は straight の許しと同じ
    let expected = d.composite(d.bounds()).unwrap();
    let cpu: Vec<u8> = expected
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|c| match c[3] {
            0 => [0, 0, 0, 0],
            255 => [c[0], c[1], c[2], 255],
            a => [mul(c[0], a), mul(c[1], a), mul(c[2], a), a],
        })
        .collect();
    let max = max_diff(&cpu, &p);
    eprintln!("乗算済みの表示と CPU の変換: 最大差 {max}");
    assert!(max <= TOLERANCE + 1, "最大差 {max}");
}

#[test]
fn shared_device_and_limited_devices() {
    gpu_lease::lease();
    let instance =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let Ok(adapter) =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
    else {
        require_gpu::skipped("共有デバイスの試験", "アダプターなし");
        return;
    };
    let info = adapter.get_info();
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let painter = GpuPainter::from_device(
        info.clone(),
        device.clone(),
        queue.clone(),
        Options::default(),
    )
    .unwrap();
    assert_eq!(painter.adapter_info().name, info.name);
    let mut g = ResidentCompositor::with_gpu(painter, ResidentOptions::default()).unwrap();
    let mut d = doc();
    let mut rng = Rng(37);
    let a = d.add_layer("a").unwrap();
    paint(&mut d, a, &mut rng, &[255, 90]);
    check(&mut g, &d, "呼び手のデバイスで合成");
    // 表示のテクスチャは呼び手のデバイスのものなので、呼び手が同じデバイスでそのまま読める（別のデバイスなら検証に失敗する）
    let display = g.display().unwrap();
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * 37,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: display.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(37),
            },
        },
        wgpu::Extent3d {
            width: 53,
            height: 37,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    assert!(
        pollster::block_on(scope.pop()).is_none(),
        "呼び手のデバイスで表示のテクスチャを使えた"
    );
    drop(g);
    // compute の使えないデバイス（WebGL2 並みの上限）は、落ちずに理由つきで断る
    let Ok((small_device, small_queue)) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
            ..Default::default()
        }))
    else {
        eprintln!("上限の低いデバイスを作れないので、断りの試験をスキップ");
        return;
    };
    let refused = GpuPainter::from_device(info, small_device, small_queue, Options::default());
    assert!(refused.is_err(), "compute の上限が 0 のデバイスは断る");
    eprintln!("compute の使えないデバイス: {}", refused.err().unwrap());
}

#[test]
fn budget_and_device_limits_refuse_with_a_reason() {
    // 予算: 表示と 1 束が入らない
    let Some(mut tiny) = gpu_with(ResidentOptions {
        resident_budget_bytes: 1 << 10,
        ..Default::default()
    }) else {
        return;
    };
    let mut d = doc();
    let a = d.add_layer("a").unwrap();
    d.set_pixel(a, 1, 1, Rgba8::new(1, 2, 3, 255)).unwrap();
    let e = tiny.update(&d, Channel::Color).unwrap_err();
    assert!(e.to_string().contains("予算"), "{e}");
    assert!(tiny.display().is_err());
    let again = tiny.update(&d, Channel::Color).unwrap_err();
    assert!(
        again.to_string().contains("再作成"),
        "失敗した常駐は作り直すまで使えない: {again}"
    );

    // 表示テクスチャがデバイスの上限を超える
    let Some(mut g) = gpu() else { return };
    let wide = Document::with_tile_size(20_000, 16, 16).unwrap();
    let e = g.update(&wide, Channel::Color).unwrap_err();
    assert!(e.to_string().contains("上限"), "{e}");

    // 面を増やして（レイヤーとマスク）予算が足りなくなる。常駐の予算は面の数に比例して束を小さくし、入らなければ断る
    let options = ResidentOptions {
        resident_budget_bytes: 4 * 16 * 16 * 4 * 2 * 3 + 53 * 37 * 4 + 400,
        batch_tiles: 1,
        ..Default::default()
    };
    let Some(mut small) = gpu_with(options) else {
        return;
    };
    let mut d = doc();
    let mut rng = Rng(41);
    let l = d.add_layer("a").unwrap();
    paint(&mut d, l, &mut rng, &[255]);
    let stats = small.update(&d, Channel::Color).unwrap();
    assert!(stats.resident_bytes <= options.resident_budget_bytes);
    for k in 0..6 {
        let l = d.add_layer("b").unwrap();
        paint(&mut d, l, &mut rng, &[255, 100]);
        d.add_layer_mask(l).unwrap();
        paint_mask(&mut d, l, &mut rng);
        let _ = k;
    }
    let e = small.update(&d, Channel::Color).unwrap_err();
    assert!(
        e.to_string().contains("予算"),
        "面が増えて 1 束も入らない: {e}"
    );
}

/// 予算の内側で、常駐が全部は入らなくても（追い出して）正しい合成になる。
#[test]
fn eviction_under_a_small_budget_keeps_the_display_correct() {
    // 53×37 の表示 7844、タイル 16² の 4 面の 1 束: 作業域 4096 の入力など。常駐はごく少数。
    let options = ResidentOptions {
        resident_budget_bytes: 53 * 37 * 4 + 4 * 1024 * 4 * 2 + 6 * 1024 * 2 + 600,
        batch_tiles: 1,
        ..Default::default()
    };
    let Some(mut g) = gpu_with(options) else {
        return;
    };
    let mut d = doc();
    let mut rng = Rng(43);
    for k in 0..3 {
        let l = d.add_layer("レイヤー").unwrap();
        paint(&mut d, l, &mut rng, &[255, 90, 200]);
        if k == 1 {
            d.add_layer_mask(l).unwrap();
            paint_mask(&mut d, l, &mut rng);
        }
    }
    let stats = g.update(&d, Channel::Color).unwrap();
    assert!(
        stats.evicted_tiles > 0,
        "予算より多く、追い出した: {stats:?}"
    );
    assert!(stats.resident_bytes <= options.resident_budget_bytes);
    check(&mut g, &d, "予算が小さくても合成は同じ");
    assert!(g.stats().resident_bytes <= options.resident_budget_bytes);
}

/// 結合や隠れたグループの出入りも、変化の記録のタイルだけで全面の合成と同じになる。
#[test]
fn merges_and_hidden_group_moves_follow_the_change_record() {
    let Some(mut g) = gpu() else { return };
    let mut d = doc();
    let mut rng = Rng(47);
    let ids: Vec<LayerId> = (0..5)
        .map(|k| {
            let l = d.add_layer("レイヤー").unwrap();
            paint(&mut d, l, &mut rng, &[255, 160, 60][k % 3..]);
            d.set_layer_opacity(l, 0.9 - 0.1 * k as f64, false).unwrap();
            l
        })
        .collect();
    check(&mut g, &d, "初め");
    let hidden = d.add_group("隠す", None).unwrap();
    d.set_layer_visible(hidden, false).unwrap();
    check(&mut g, &d, "空の隠したグループ");
    d.move_layer_to(ids[2], Some(hidden), 0).unwrap();
    check(&mut g, &d, "隠したグループへ入れる（見えなくなる）");
    d.move_layer_to(ids[2], None, 1).unwrap();
    check(&mut g, &d, "隠したグループから出す（見える）");
    d.move_layer_to(ids[3], Some(hidden), 0).unwrap();
    d.set_layer_visible(hidden, true).unwrap();
    check(&mut g, &d, "グループを見せる");
    d.set_layer_visible(hidden, false).unwrap();
    d.ungroup(hidden).unwrap();
    check(&mut g, &d, "隠したグループを解く（中身は見える）");
    d.merge_down(ids[1], 255).unwrap();
    check(&mut g, &d, "下へ結合");
    d.merge_layers(&[ids[3], ids[4]], 255).unwrap();
    check(&mut g, &d, "選んだレイヤーを結合");
    d.undo().unwrap();
    check(&mut g, &d, "結合の取消");
    d.redo().unwrap();
    check(&mut g, &d, "結合のやり直し");
    d.merge_visible("結合", 255).unwrap();
    check(&mut g, &d, "表示しているレイヤーを結合");
    d.undo().unwrap();
    check(&mut g, &d, "取消");
}

/// 見積もり（GPU に触れない）が、全部が常駐したときに `update` が数える量と同じ。予算を超える文書・上限を超える文書も見積もれる。
#[test]
fn requirements_match_what_update_counts_and_flag_documents_over_the_budget() {
    let limits = wgpu::Limits::default();
    let options = ResidentOptions::default();
    // 疎な 4096²: 描いた 2 タイルのレイヤーと、マスクの 1 タイル
    let mut d = Document::with_tile_size(4096, 4096, 128).unwrap();
    let l = d.add_layer("疎").unwrap();
    d.set_pixel(l, 130, 5, Rgba8::new(10, 20, 30, 255)).unwrap();
    d.set_pixel(l, 4000, 4000, Rgba8::new(40, 50, 60, 255))
        .unwrap();
    d.add_layer_mask(l).unwrap();
    d.set_mask_pixel(l, 130, 5, 200).unwrap();
    let need = resident_requirements(&d, Channel::Color, &options, &limits).unwrap();
    assert_eq!(
        need.tile_bytes,
        3 * 128 * 128 * 4 * 2,
        "レイヤーの 2 タイルとマスクの 1 タイル、GPU と CPU のコピー"
    );
    assert!(need.total_bytes() < options.resident_budget_bytes);
    if let Some(mut g) = gpu() {
        let stats = g.update(&d, Channel::Color).unwrap();
        assert_eq!(stats.cached_tiles, 3);
        assert_eq!(
            stats.resident_bytes,
            need.total_bytes(),
            "見積もりと実際の数え方は同じ"
        );
    }
    // 密な文書: レイヤーを足すほど増える。見えないレイヤー・無いタイルは数えない
    let mut dense = Document::with_tile_size(256, 128, 16).unwrap();
    let mut rng = Rng(5);
    for _ in 0..3 {
        let l = dense.add_layer("密").unwrap();
        paint(&mut dense, l, &mut rng, &[255]);
    }
    let all = resident_requirements(&dense, Channel::Color, &options, &limits).unwrap();
    assert_eq!(all.tile_bytes, 3 * (16 * 8) * 16 * 16 * 4 * 2);
    let hidden = dense.layers()[0].id();
    dense.set_layer_visible(hidden, false).unwrap();
    let less = resident_requirements(&dense, Channel::Color, &options, &limits).unwrap();
    assert_eq!(
        less.tile_bytes,
        all.tile_bytes / 3 * 2,
        "見えないレイヤーは上げないので数えない"
    );
    if let Some(mut g) = gpu() {
        // 隠す前に常駐させてから隠しても、常駐は見積もり（見えるレイヤーだけ）へ戻る（隠したレイヤーのタイルを抱え続けない）
        dense.set_layer_visible(hidden, true).unwrap();
        let stats = g.update(&dense, Channel::Color).unwrap();
        assert_eq!(stats.resident_bytes, all.total_bytes());
        assert_eq!(stats.cached_tiles, 3 * 16 * 8);
        dense.set_layer_visible(hidden, false).unwrap();
        let stats = g.update(&dense, Channel::Color).unwrap();
        assert_eq!(stats.cached_tiles, 2 * 16 * 8);
        assert_eq!(stats.resident_bytes, less.total_bytes());
    }
    // 予算を超える: 描いたタイルの分（GPU と CPU のコピー）だけでも予算に入らなければ、見積もりが予算より大きい
    let small = ResidentOptions {
        resident_budget_bytes: less.tile_bytes,
        ..options
    };
    let over = resident_requirements(&dense, Channel::Color, &small, &limits).unwrap();
    assert!(over.total_bytes() > small.resident_budget_bytes, "{over:?}");
    let tiny = ResidentOptions {
        resident_budget_bytes: 100,
        ..options
    };
    let none = resident_requirements(&dense, Channel::Color, &tiny, &limits).unwrap();
    assert_eq!(none.fixed_bytes, u64::MAX);
    assert_eq!(none.total_bytes(), u64::MAX);
    // 上限・扱えない文書は Err
    let wide = Document::with_tile_size(20_000, 16, 16).unwrap();
    let e = resident_requirements(&wide, Channel::Color, &options, &limits).unwrap_err();
    assert!(e.to_string().contains("上限"), "{e}");
    let e = resident_requirements(&dense, Channel::from_index(40).unwrap(), &options, &limits)
        .unwrap_err();
    assert!(e.to_string().contains("チャンネル"), "{e}");
}
