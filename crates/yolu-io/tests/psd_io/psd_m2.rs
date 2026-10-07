//! PSD の写し（core の文書 ⇔ PSD）の M2 のレイヤー: マスク・グループ・塗りつぶし・調整・クリッピング・チャンネルごとの合成・ロック。
//! C# の PsdLayerFeatureTests・PsdGroupTests・PsdAdjustmentTests・PsdFillTests・PsdLockTests と同じ道筋を core の文書から確かめる
//! （C# 正解のバイト列との照合は `psd_golden.rs`）。Photoshop・CLIP STUDIO の実物のファイルは持ち込まず、試験の中で一から組む。
use std::fmt::Write as _;
use yolu_core::{
    AdjustmentSettings, BlendMode as CoreBlend, Channel, ChannelBlend, Document, LayerId,
    LayerKind as CoreKind, LayerLocks, Rgba8,
};
use yolu_io::psd::{
    self, Adjustment, BlendMode, CompatibilityMode as Mode, Document as Psd, Layer, LayerKind,
    Limits, Mask,
};

const W: u32 = 24;
const H: u32 = 16;

fn new_doc() -> Document {
    Document::with_tile_size(W, H, 8).unwrap()
}
fn gradient(x: u32, y: u32) -> Rgba8 {
    Rgba8::new(
        (x * 10 + 15) as u8,
        (y * 14 + 20) as u8,
        (200 - x * 5) as u8,
        255,
    )
}
fn soft(x: u32, y: u32) -> Rgba8 {
    Rgba8::new(
        (255 - x * 9) as u8,
        (x * 7 + y * 5) as u8,
        (y * 13 + 40) as u8,
        if x < 3 { 0 } else { (60 + x * 8 + y * 3) as u8 },
    )
}
fn raster(d: &mut Document, name: &str, pixel: fn(u32, u32) -> Rgba8) -> LayerId {
    let id = d.add_layer(name).unwrap();
    for y in 0..H {
        for x in 0..W {
            let c = pixel(x, y);
            if c != Rgba8::TRANSPARENT {
                d.set_pixel(id, x, y, c).unwrap();
            }
        }
    }
    id
}
fn hide_row(d: &mut Document, id: LayerId, y: u32, amount: u8) {
    d.add_layer_mask(id).unwrap();
    for x in 0..W {
        d.set_mask_pixel(id, x, y, amount).unwrap();
    }
}
fn fill(d: &mut Document, name: &str, c: Rgba8) -> LayerId {
    d.add_fill_layer(name, &[(Channel::Color, c)], None)
        .unwrap()
}
fn adjust(d: &mut Document, name: &str, s: AdjustmentSettings) -> LayerId {
    d.add_adjustment_layer(name, s, None, None).unwrap()
}
fn exact_adjustments() -> [AdjustmentSettings; 3] {
    [
        AdjustmentSettings::invert(),
        AdjustmentSettings::levels(
            20.0 / 255.0,
            230.0 / 255.0,
            1.37,
            10.0 / 255.0,
            240.0 / 255.0,
        )
        .unwrap(),
        AdjustmentSettings::hue_saturation(-73.0, 0.42, -0.18).unwrap(),
    ]
}
fn opacity(d: &mut Document, id: LayerId, byte: u8) {
    d.set_layer_opacity(id, f64::from(byte) / 255.0, false)
        .unwrap()
}
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}

/// 往復で変わってはいけない中身の文字列（レイヤーの並び・入れ子・属性・マスク・調整・塗りつぶし・画素・合成）。ロックは、すべてが立てば
/// 下の個別のロックを書かない（効くロックは同じ）ので、すべてだけにそろえる。
fn snapshot(d: &Document) -> String {
    let mut s = String::new();
    for (i, l) in d.layers().iter().enumerate() {
        let parent = l
            .parent()
            .and_then(|p| d.layer_index(p))
            .map_or(-1, |i| i as i64);
        let locks = if l.locks().contains(LayerLocks::ALL) {
            LayerLocks::ALL.bits()
        } else {
            l.locks().bits()
        };
        write!(
            s,
            "{i} {:?} kind={:?} parent={parent} visible={} opacity={} mode={:?} clip={} locks={locks}",
            l.name(),
            l.kind(),
            l.visible(),
            (l.opacity() * 255.0).round_ties_even(),
            l.blend_mode(),
            l.clipping(),
        )
        .unwrap();
        match l.mask() {
            None => s.push_str(" mask=-"),
            Some(m) => write!(
                s,
                " mask={}/{}/{}/{:016x}",
                m.enabled(),
                m.inverted(),
                (m.density() * 255.0).round_ties_even(),
                fnv(&m.surface().to_canvas_bytes())
            )
            .unwrap(),
        }
        match l.kind() {
            CoreKind::Raster => write!(
                s,
                " pixels={:016x}",
                fnv(&l.surface(Channel::Color).unwrap().to_canvas_bytes())
            )
            .unwrap(),
            CoreKind::Fill => write!(s, " fill={:?}", l.fill_value(Channel::Color)).unwrap(),
            CoreKind::Adjustment => {
                let a = l.adjustment().unwrap();
                write!(
                    s,
                    " adjustment={:?}/{:x}/{:x}/{:x}/{:x}/{:x}/{:x}/{:x}/{:x}",
                    a.kind(),
                    a.input_black().to_bits(),
                    a.input_white().to_bits(),
                    a.gamma().to_bits(),
                    a.output_black().to_bits(),
                    a.output_white().to_bits(),
                    a.hue().to_bits(),
                    a.saturation().to_bits(),
                    a.lightness().to_bits()
                )
                .unwrap()
            }
            CoreKind::Group => {}
        }
        s.push('\n');
    }
    write!(
        s,
        "composite={:016x}",
        fnv(&d.composite(d.bounds()).unwrap())
    )
    .unwrap();
    s
}

/// core → PSD → バイト列 → 読み → core。PSD の読みに診断が無い・原本の書き戻しが同じバイト列・もう一度書き出すと同じ PSD になる、まで確かめる。
fn round_trip(core: &Document) -> (Document, Vec<u8>) {
    let projected = Psd::from_core(core).unwrap();
    let bytes = psd::write(&projected, &Limits::default()).unwrap();
    let read = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(
        read.mode(),
        Mode::EditableRaster,
        "{:?}",
        read.diagnostics()
    );
    assert!(read.diagnostics().is_empty(), "{:?}", read.diagnostics());
    let doc = read.document().unwrap();
    assert_eq!((doc.width, doc.height), (projected.width, projected.height));
    assert_eq!(
        doc.layers, projected.layers,
        "読み直したレイヤーは書いたレイヤーと同じ（統合画像は読みでは持たない）"
    );
    assert_eq!(
        psd::write_edited(&read, doc, &Limits::default()).unwrap(),
        bytes,
        "原本の書き戻しは同じバイト列"
    );
    let back = read.to_core().unwrap();
    assert!(!back.can_undo(), "取り込みは Undo の履歴に残さない");
    assert_eq!(
        Psd::from_core(&back).unwrap(),
        projected,
        "書き出し直すと同じ PSD"
    );
    (back, bytes)
}
fn refusal(core: &Document) -> String {
    Psd::from_core(core).unwrap_err().to_string()
}

#[test]
fn groups_fills_adjustments_masks_and_clipping_round_trip_with_the_same_composite() {
    let mut d = new_doc();
    raster(&mut d, "bg", gradient);
    let paint = fill(&mut d, "paint", Rgba8::new(200, 100, 50, 255));
    opacity(&mut d, paint, 128);
    d.set_layer_blend_mode(paint, CoreBlend::Multiply).unwrap();
    hide_row(&mut d, paint, 5, 200);
    d.set_layer_mask_density(paint, 160.0 / 255.0, false)
        .unwrap();
    let shape = raster(&mut d, "shape", soft);
    let [invert, levels, hue] = exact_adjustments();
    let clipped = adjust(&mut d, "clipped levels", levels.clone());
    d.set_layer_clipping(clipped, true).unwrap();
    d.set_layer_blend_mode(clipped, CoreBlend::Overlay).unwrap();
    let b = raster(&mut d, "b", soft);
    let h = adjust(&mut d, "hue", hue);
    let i = adjust(&mut d, "invert", invert);
    d.set_layer_clipping(i, true).unwrap();
    opacity(&mut d, i, 100);
    let isolated = d.group_layers(&[b, h, i], "isolated").unwrap();
    d.set_layer_blend_mode(isolated, CoreBlend::Multiply)
        .unwrap();
    let a = raster(&mut d, "a", soft);
    let l2 = adjust(&mut d, "levels", levels);
    let pass = d.group_layers(&[a, l2], "pass").unwrap();
    opacity(&mut d, pass, 190);
    hide_row(&mut d, pass, 9, 90);
    let empty = d.add_group("empty", None).unwrap();
    d.set_layer_visible(empty, false).unwrap();
    let hidden = fill(&mut d, "hidden", Rgba8::new(1, 2, 3, 255));
    d.set_layer_visible(hidden, false).unwrap();
    let _ = shape;
    let (back, bytes) = round_trip(&d);
    assert_eq!(snapshot(&back), snapshot(&d));
    assert_eq!(
        back.composite(back.bounds()).unwrap(),
        d.composite(d.bounds()).unwrap(),
        "再読み込みの合成はバイト一致"
    );
    // PSD の形: フォルダーの区切り・調整・塗りつぶしのブロックが入っている
    let text = String::from_utf8_lossy(&bytes);
    for key in ["8BIMlsct", "8BIMSoCo", "8BIMlevl", "8BIMhue2", "8BIMnvrt"] {
        assert!(text.contains(key), "{key}");
    }
}

#[test]
fn every_blend_mode_on_groups_and_fills_and_adjustments_round_trips() {
    for mode in CoreBlend::LAYER_MODES {
        let mut d = new_doc();
        raster(&mut d, "bg", gradient);
        let f = fill(&mut d, "fill", Rgba8::new(20, 180, 90, 255));
        d.set_layer_blend_mode(f, mode).unwrap();
        let a = adjust(&mut d, "adjust", exact_adjustments()[2].clone());
        d.set_layer_blend_mode(a, mode).unwrap();
        opacity(&mut d, a, 150);
        let inner = raster(&mut d, "inner", soft);
        let g = d.group_layers(&[inner], "group").unwrap();
        d.set_layer_blend_mode(g, mode).unwrap();
        opacity(&mut d, g, 200);
        let (back, _) = round_trip(&d);
        assert_eq!(snapshot(&back), snapshot(&d), "{mode:?}");
    }
    // 通過のグループ
    let mut d = new_doc();
    raster(&mut d, "bg", gradient);
    let inner = raster(&mut d, "inner", soft);
    d.set_layer_blend_mode(inner, CoreBlend::Screen).unwrap();
    let g = d.group_layers(&[inner], "pass").unwrap();
    assert_eq!(d.layer(g).unwrap().blend_mode(), CoreBlend::PassThrough);
    let (back, _) = round_trip(&d);
    assert_eq!(snapshot(&back), snapshot(&d));
}

#[test]
fn each_adjustment_round_trips_with_opacity_mask_clipping_and_hidden() {
    for settings in exact_adjustments() {
        let mut d = new_doc();
        raster(&mut d, "bg", gradient);
        let plain = adjust(&mut d, "plain", settings.clone());
        opacity(&mut d, plain, 160);
        hide_row(&mut d, plain, 5, 200);
        raster(&mut d, "shape", soft);
        let clipped = adjust(&mut d, "clipped", settings.clone());
        d.set_layer_clipping(clipped, true).unwrap();
        d.set_layer_blend_mode(clipped, CoreBlend::Overlay).unwrap();
        let hidden = adjust(&mut d, "hidden", AdjustmentSettings::invert());
        d.set_layer_visible(hidden, false).unwrap();
        let (back, bytes) = round_trip(&d);
        assert_eq!(snapshot(&back), snapshot(&d), "{:?}", settings.kind());
        let read = psd::read(&bytes, &Limits::default()).unwrap();
        let layers = &read.document().unwrap().layers;
        assert!(matches!(layers[3].kind, LayerKind::Adjustment(_)));
        assert!(layers[3].mask.is_some());
        assert!(layers[1].clipping && layers[1].blend_mode == BlendMode::Overlay);
    }
}

#[test]
fn an_adjustment_as_a_clip_base_keeps_the_core_rule() {
    // 調整は下地にならず、その上のクリッピングは描かれない（core の合成と PSD の参照合成が同じ）
    let mut d = new_doc();
    raster(&mut d, "bg", gradient);
    adjust(&mut d, "base", exact_adjustments()[2].clone());
    let c = raster(&mut d, "clipped to an adjustment", |_, _| {
        Rgba8::new(255, 0, 0, 255)
    });
    d.set_layer_clipping(c, true).unwrap();
    let (back, _) = round_trip(&d);
    assert_eq!(snapshot(&back), snapshot(&d));
}

#[test]
fn fill_layers_round_trip_as_solid_colour_blocks_and_stay_clip_bases() {
    let mut d = new_doc();
    raster(&mut d, "bg", gradient);
    let f = fill(&mut d, "base fill", Rgba8::new(10, 200, 30, 255));
    hide_row(&mut d, f, 3, 255);
    let c = raster(&mut d, "clipped", soft);
    d.set_layer_clipping(c, true).unwrap();
    let c2 = fill(&mut d, "clipped fill", Rgba8::new(250, 20, 20, 255));
    d.set_layer_clipping(c2, true).unwrap();
    opacity(&mut d, c2, 90);
    let (back, bytes) = round_trip(&d);
    assert_eq!(snapshot(&back), snapshot(&d));
    // 塗りつぶしは画素に焼かない（SoCo のまま。画素は持たない）
    let read = psd::read(&bytes, &Limits::default()).unwrap();
    let layers = &read.document().unwrap().layers;
    assert_eq!(layers[0].kind, LayerKind::SolidColor([250, 20, 20]));
    assert!(layers[0].pixels_rgba.is_empty() && layers[0].width == 0);
    assert_eq!(layers[2].kind, LayerKind::SolidColor([10, 200, 30]));
}

#[test]
fn nested_groups_round_trip_to_the_depth_budget_and_no_further() {
    let mut d = new_doc();
    let mut current = raster(&mut d, "leaf", soft);
    for n in 0..32 {
        current = d.group_layers(&[current], &format!("g{n}")).unwrap();
    }
    let (back, _) = round_trip(&d);
    assert_eq!(snapshot(&back), snapshot(&d));
    assert_eq!(back.depth_of(back.layers()[0].id()).unwrap(), 32);
    // もう 1 段で PSD の入れ子の予算（32 段）を超える。書かず予算の理由を言う
    d.group_layers(&[current], "too deep").unwrap();
    let err = Psd::from_core(&d).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
}

#[test]
fn mask_rectangles_use_the_smaller_default_colour_and_disabled_density_survive() {
    // 一部だけ隠す: 見える所が多いので既定 255・隠す所の矩形だけ書く
    let mut d = new_doc();
    let a = raster(&mut d, "mostly visible", gradient);
    hide_row(&mut d, a, 5, 255);
    d.set_layer_mask_enabled(a, false).unwrap();
    d.set_layer_mask_density(a, 40.0 / 255.0, false).unwrap();
    // ほとんど隠す: 既定 0・見える所の矩形だけ書く
    let b = raster(&mut d, "mostly hidden", soft);
    d.add_layer_mask(b).unwrap();
    for y in 0..H {
        for x in 0..W {
            if !(4..9).contains(&x) || !(2..6).contains(&y) {
                d.set_mask_pixel(b, x, y, 255).unwrap();
            }
        }
    }
    // 変えないマスクと全部隠すマスクも 1×1 の矩形で書く
    let c = raster(&mut d, "neutral", gradient);
    d.add_layer_mask(c).unwrap();
    let e = raster(&mut d, "all hidden", gradient);
    d.add_layer_mask(e).unwrap();
    for y in 0..H {
        for x in 0..W {
            d.set_mask_pixel(e, x, y, 255).unwrap();
        }
    }
    let projected = Psd::from_core(&d).unwrap();
    let masks: Vec<&Mask> = projected
        .layers
        .iter()
        .rev()
        .map(|l| l.mask.as_ref().unwrap())
        .collect();
    assert_eq!(
        (
            masks[0].default_color,
            masks[0].left,
            masks[0].top,
            masks[0].width,
            masks[0].height
        ),
        (255, 0, H as i32 - 6, W, 1),
        "上下が逆（PSD は上から）"
    );
    assert!(!masks[0].enabled && masks[0].density == 40);
    assert_eq!(
        (
            masks[1].default_color,
            masks[1].left,
            masks[1].top,
            masks[1].width,
            masks[1].height
        ),
        (0, 4, H as i32 - 6, 5, 4)
    );
    assert_eq!(
        (masks[2].default_color, masks[2].width, masks[2].height),
        (255, 1, 1)
    );
    assert_eq!(
        (masks[3].default_color, masks[3].width, masks[3].height),
        (0, 1, 1)
    );
    let (back, _) = round_trip(&d);
    assert_eq!(snapshot(&back), snapshot(&d));
}

#[test]
fn locks_round_trip_on_every_kind_and_a_locked_folder_locks_its_contents() {
    for lock in [
        LayerLocks::TRANSPARENCY,
        LayerLocks::PIXELS,
        LayerLocks::POSITION,
        LayerLocks::ALL,
        LayerLocks::ALL | LayerLocks::TRANSPARENCY | LayerLocks::PIXELS | LayerLocks::POSITION,
    ] {
        let mut d = new_doc();
        let a = raster(&mut d, "a", gradient);
        let f = fill(&mut d, "f", Rgba8::new(1, 2, 3, 255));
        let j = adjust(&mut d, "j", AdjustmentSettings::invert());
        let inner = raster(&mut d, "inner", soft);
        let g = d.group_layers(&[inner], "g").unwrap();
        for id in [a, f, j, g] {
            d.set_layer_locks(id, lock).unwrap();
        }
        d.clear_history().unwrap();
        let (back, bytes) = round_trip(&d);
        assert_eq!(snapshot(&back), snapshot(&d), "{lock:?}");
        for (x, y) in d.layers().iter().zip(back.layers()) {
            assert_eq!(
                d.effective_locks(x.id()).unwrap(),
                back.effective_locks(y.id()).unwrap(),
                "{} の効くロックは同じ",
                x.name()
            );
        }
        let text = String::from_utf8_lossy(&bytes);
        assert_eq!(text.matches("8BIMlspf").count(), 4, "{lock:?}");
    }
    let mut d = new_doc();
    raster(&mut d, "a", gradient);
    let (_, bytes) = round_trip(&d);
    assert!(
        !String::from_utf8_lossy(&bytes).contains("lspf"),
        "ロックが無ければ lspf を書かない"
    );
}

#[test]
fn imported_locks_are_in_core_without_history_and_the_project_stores_them() {
    let psd = Psd {
        width: 2,
        height: 1,
        layers: vec![
            Layer {
                id: 2,
                name: "b".into(),
                width: 2,
                height: 1,
                pixels_rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
                locks: 0x8000_0000,
                ..Layer::default()
            },
            Layer {
                id: 1,
                name: "a".into(),
                width: 2,
                height: 1,
                pixels_rgba: vec![7, 8, 9, 255, 10, 11, 12, 255],
                locks: 1 | 4,
                ..Layer::default()
            },
        ],
        composite_rgba: None,
    };
    assert!(psd.core_issues().is_empty(), "core はロックを持てる");
    let core = psd.to_core().unwrap();
    assert!(!core.can_undo(), "ロックは読み込みで入れ、編集にしない");
    assert_eq!(
        core.layers()[0].locks(),
        LayerLocks::TRANSPARENCY | LayerLocks::POSITION
    );
    assert_eq!(core.layers()[1].locks(), LayerLocks::ALL);
    // .ylp の正本にも書ける（版 12 の並び）。書いて読み戻しても同じロックで、PSD に書き出し直しても同じ lspf
    let native = yolu_io::NativeDocument::from_core(&core).unwrap();
    assert!(native.core_issues().is_empty());
    let again = native.to_core().unwrap();
    assert_eq!(
        again.layers()[0].locks(),
        LayerLocks::TRANSPARENCY | LayerLocks::POSITION
    );
    assert_eq!(again.layers()[1].locks(), LayerLocks::ALL);
    let back = Psd::from_core(&again).unwrap();
    assert_eq!(
        back.layers.iter().map(|l| l.locks).collect::<Vec<_>>(),
        psd.layers.iter().map(|l| l.locks).collect::<Vec<_>>()
    );
}

#[test]
fn the_colour_blend_of_a_layer_is_written_whatever_the_other_channels_say() {
    // Color の合成がレイヤーの値と違うとき、Color のほうを書く（C# が書き出すチャンネルの値で書くのと同じ）。ほかのチャンネルの設定は、その
    // チャンネルの PSD の値になるので、Color の PSD には関わらず、断らない
    let mut d = new_doc();
    raster(&mut d, "bg", gradient);
    let a = raster(&mut d, "a", soft);
    opacity(&mut d, a, 200);
    d.set_channel_blend(
        a,
        Channel::Color,
        ChannelBlend::new(Some(CoreBlend::Multiply), Some(100.0 / 255.0)),
        false,
    )
    .unwrap();
    let projected = Psd::from_core(&d).unwrap();
    assert_eq!(
        (projected.layers[0].blend_mode, projected.layers[0].opacity),
        (BlendMode::Multiply, 100)
    );
    for (channel, blend) in [
        (
            Channel::Roughness,
            ChannelBlend::new(Some(CoreBlend::Multiply), Some(100.0 / 255.0)),
        ),
        (
            Channel::Metallic,
            ChannelBlend::new(Some(CoreBlend::Screen), None),
        ),
    ] {
        d.set_channel_blend(a, channel, blend, false).unwrap();
    }
    let (back, _) = round_trip(&d);
    assert_eq!(back.layers()[1].blend_mode(), CoreBlend::Multiply);
    assert_eq!(
        back.layers()[1].channel_blends().count(),
        0,
        "読み込み直すとレイヤーの値になる"
    );
    // グループも Color の値で書く。ほかのチャンネルの実効の値が違っても断らない
    let inner = raster(&mut d, "inner", soft);
    let g = d.group_layers(&[inner], "group").unwrap();
    d.set_channel_opacity(g, Channel::Emission, Some(64.0 / 255.0), false)
        .unwrap();
    assert_eq!(Psd::from_core(&d).unwrap().layers[0].opacity, 255);
    d.set_channel_opacity(g, Channel::Color, Some(64.0 / 255.0), false)
        .unwrap();
    assert_eq!(Psd::from_core(&d).unwrap().layers[0].opacity, 64);
    let (back, _) = round_trip(&d);
    let group = back.layers().iter().find(|l| l.is_group()).unwrap();
    assert_eq!(group.opacity_in(Channel::Roughness), 64.0 / 255.0);
    assert_eq!(group.channel_blends().count(), 0);
}

#[test]
fn an_adjustments_colour_opacity_is_written_whatever_the_other_channels_it_acts_on_say() {
    let mut d = new_doc();
    raster(&mut d, "bg", gradient);
    let adj = adjust(&mut d, "adjust", AdjustmentSettings::invert());
    assert!(d.layer(adj).unwrap().enabled_channels().len() > 1);
    d.set_channel_opacity(adj, Channel::Color, Some(150.0 / 255.0), false)
        .unwrap();
    assert_eq!(Psd::from_core(&d).unwrap().layers[0].opacity, 150);
    // 効くチャンネルを Color だけにしても同じ。読み込み直すと効く標準のチャンネル全部になる（調整の効くチャンネルは持ち越さない。
    // C# の取り込みと同じ）
    for channel in d.layer(adj).unwrap().enabled_channels() {
        if channel != Channel::Color {
            d.set_channel_enabled(adj, channel, false).unwrap();
        }
    }
    assert_eq!(Psd::from_core(&d).unwrap().layers[0].opacity, 150);
}

/// 断りの事例: 名前・文書の組み方・断りの文に入っているべき語。
type RefusedCase<'a> = (&'a str, Box<dyn Fn(&mut Document)>, &'a [&'a str]);

#[test]
fn the_strict_export_refuses_what_psd_has_no_form_for_with_the_layer_name_and_nothing_is_flattened()
{
    let cases: Vec<RefusedCase> = vec![
        (
            "inverted mask",
            Box::new(|d| {
                let a = raster(d, "inv", gradient);
                d.add_layer_mask(a).unwrap();
                d.set_layer_mask_inverted(a, true).unwrap();
            }),
            &["「inv」", "反転"],
        ),
        (
            "clipped group",
            Box::new(|d| {
                raster(d, "bg", gradient);
                let c = raster(d, "inner", soft);
                let g = d.group_layers(&[c], "clipped group").unwrap();
                d.set_layer_clipping(g, true).unwrap();
            }),
            &["「clipped group」", "クリッピング"],
        ),
        (
            "translucent fill",
            Box::new(|d| {
                raster(d, "bg", gradient);
                fill(d, "glass", Rgba8::new(1, 2, 3, 128));
            }),
            &["「glass」", "半透明"],
        ),
        (
            "levels between the steps",
            Box::new(|d| {
                raster(d, "bg", gradient);
                adjust(
                    d,
                    "between",
                    AdjustmentSettings::levels(0.3, 1.0, 1.0, 0.0, 1.0).unwrap(),
                );
            }),
            &["「between」", "刻み"],
        ),
        (
            "gamma between the hundredths",
            Box::new(|d| {
                raster(d, "bg", gradient);
                adjust(
                    d,
                    "gamma",
                    AdjustmentSettings::levels(0.0, 1.0, 1.234, 0.0, 1.0).unwrap(),
                );
            }),
            &["「gamma」", "刻み"],
        ),
        (
            "hue between the degrees",
            Box::new(|d| {
                raster(d, "bg", gradient);
                adjust(
                    d,
                    "hue",
                    AdjustmentSettings::hue_saturation(10.5, 0.0, 0.0).unwrap(),
                );
            }),
            &["「hue」", "刻み"],
        ),
        (
            "saturation between the percents",
            Box::new(|d| {
                raster(d, "bg", gradient);
                adjust(
                    d,
                    "sat",
                    AdjustmentSettings::hue_saturation(0.0, 0.333, 0.0).unwrap(),
                );
            }),
            &["「sat」", "刻み"],
        ),
        (
            "levels outside the PSD input range",
            Box::new(|d| {
                raster(d, "bg", gradient);
                adjust(
                    d,
                    "narrow",
                    AdjustmentSettings::levels(0.0, 1.0 / 255.0, 1.0, 0.0, 1.0).unwrap(),
                );
            }),
            &["「narrow」", "0〜253"],
        ),
        (
            "inside a group",
            Box::new(|d| {
                let a = raster(d, "ok", gradient);
                let m = raster(d, "masked inside", soft);
                d.add_layer_mask(m).unwrap();
                d.set_layer_mask_inverted(m, true).unwrap();
                d.group_layers(&[a, m], "folder").unwrap();
            }),
            &["「masked inside」"],
        ),
    ];
    for (label, build, needles) in cases {
        let mut d = new_doc();
        build(&mut d);
        let why = refusal(&d);
        for needle in needles {
            assert!(why.contains(needle), "{label}: {why}");
        }
    }
}

#[test]
fn layer_and_divider_ids_survive_import_and_export() {
    let mut psd = Psd {
        width: 2,
        height: 1,
        layers: vec![
            Layer {
                id: 41,
                name: "folder".into(),
                kind: LayerKind::Group {
                    children: vec![Layer {
                        id: 17,
                        name: "inside".into(),
                        width: 2,
                        height: 1,
                        pixels_rgba: vec![9, 9, 9, 255, 0, 0, 0, 0],
                        ..Layer::default()
                    }],
                    divider_id: 4_000_123,
                },
                blend_mode: BlendMode::PassThrough,
                ..Layer::default()
            },
            Layer {
                id: 5,
                name: "fill".into(),
                kind: LayerKind::SolidColor([1, 2, 3]),
                ..Layer::default()
            },
            Layer {
                id: 6,
                name: "adjust".into(),
                kind: LayerKind::Adjustment(Adjustment::Invert),
                ..Layer::default()
            },
        ],
        composite_rgba: None,
    };
    let core = psd.to_core().unwrap();
    // 下から: adjust・fill・inside・folder（グループの中身はグループのすぐ下）
    let names: Vec<_> = core.layers().iter().map(|l| l.name()).collect();
    assert_eq!(names, ["adjust", "fill", "inside", "folder"]);
    assert_eq!(core.layers()[2].parent(), Some(core.layers()[3].id()));
    assert_eq!(core.layers()[3].parent(), None);
    let projected = Psd::from_core(&core).unwrap();
    psd.composite_rgba = projected.composite_rgba.clone();
    assert_eq!(
        projected, psd,
        "レイヤーの ID・区切りの ID・並び・入れ子が同じ"
    );
    // 同じ PSD のレイヤーの ID が重なる文書: 次の空きへ送る
    let mut d = new_doc();
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    let d = d
        .with_persistent_ids(1, &[LayerId(7 << 96), LayerId(7 << 96 | 1)])
        .unwrap();
    let _ = (a, b);
    let projected = Psd::from_core(&d).unwrap();
    assert_eq!(
        projected.layers.iter().map(|l| l.id).collect::<Vec<_>>(),
        [7, 8],
        "上のレイヤーから順に、重なれば次の空きへ"
    );
}

#[test]
fn off_canvas_pixels_and_masks_are_refused_not_cropped() {
    let layer = |left, top, mask: Option<Mask>| Layer {
        id: 1,
        name: "x".into(),
        left,
        top,
        width: 2,
        height: 1,
        pixels_rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        mask,
        ..Layer::default()
    };
    let doc = |l: Layer| Psd {
        width: 4,
        height: 2,
        layers: vec![l],
        composite_rgba: None,
    };
    let mask = |left, default_color, value| Mask {
        left,
        top: 0,
        width: 2,
        height: 1,
        default_color,
        enabled: true,
        density: 255,
        pixels: vec![value; 2],
    };
    assert!(
        doc(layer(3, 0, None)).to_core().is_err(),
        "右にはみ出す画素"
    );
    // 既定の値と同じ値だけがキャンバスの外にあるなら、切り捨てても同じなので入れる
    let inside = doc(layer(0, 0, Some(mask(3, 255, 255))));
    assert!(inside.core_issues().is_empty());
    assert!(inside.to_core().is_ok());
    // 既定の値と違う値が外にあれば、黙って切り捨てず断る
    let outside = doc(layer(0, 0, Some(mask(3, 255, 0))));
    assert_eq!(outside.core_issues().len(), 1);
    assert!(outside.core_issues()[0].contains("layers[0].mask"));
    assert!(outside.to_core().is_err());
    // グループの中のレイヤーも数える
    let nested = Psd {
        width: 4,
        height: 2,
        layers: vec![Layer {
            id: 9,
            name: "g".into(),
            kind: LayerKind::Group {
                children: vec![layer(3, 0, None)],
                divider_id: 0,
            },
            blend_mode: BlendMode::PassThrough,
            ..Layer::default()
        }],
        composite_rgba: None,
    };
    assert!(nested.core_issues()[0].starts_with("layers[0].children[0]"));
}

#[test]
fn a_mask_default_colour_zero_hides_everywhere_outside_its_rectangle() {
    let psd = Psd {
        width: 6,
        height: 4,
        layers: vec![Layer {
            id: 1,
            name: "x".into(),
            width: 6,
            height: 4,
            pixels_rgba: [10, 20, 30, 255].repeat(24),
            mask: Some(Mask {
                left: 2,
                top: 1,
                width: 2,
                height: 2,
                default_color: 0,
                enabled: true,
                density: 255,
                pixels: vec![255, 128, 0, 255],
            }),
            ..Layer::default()
        }],
        composite_rgba: None,
    };
    let core = psd.to_core().unwrap();
    let m = core.layers()[0].mask().unwrap();
    // PSD の行 1 は core の行 2、PSD の行 2 は core の行 1
    let hide = |x, y| m.surface().pixel(x, y).unwrap().a;
    assert_eq!(
        (hide(2, 2), hide(3, 2), hide(2, 1), hide(3, 1)),
        (0, 127, 255, 0)
    );
    assert_eq!(
        (hide(0, 0), hide(5, 3), hide(1, 2), hide(4, 1)),
        (255, 255, 255, 255)
    );
    let again = Psd::from_core(&core).unwrap();
    let mask = again.layers[0].mask.as_ref().unwrap();
    assert_eq!(
        (
            mask.default_color,
            mask.left,
            mask.top,
            mask.width,
            mask.height
        ),
        (0, 2, 1, 2, 2)
    );
    assert_eq!(mask.pixels, [255, 128, 0, 255]);
}

#[test]
fn exporting_leaves_the_document_untouched_and_a_stroke_blocks_it() {
    let mut d = new_doc();
    let a = raster(&mut d, "a", soft);
    hide_row(&mut d, a, 2, 100);
    let g = d.group_layers(&[a], "g").unwrap();
    d.set_layer_locks(g, LayerLocks::POSITION).unwrap();
    let before = (d.revision(), d.undo_count(), snapshot(&d));
    let _ = Psd::from_core(&d).unwrap();
    assert_eq!((d.revision(), d.undo_count(), snapshot(&d)), before);
    let stroke = d
        .begin_stroke(a, &yolu_core::BrushSettings::default())
        .unwrap();
    assert!(Psd::from_core(&d).is_err());
    d.cancel_stroke(stroke);
    assert!(Psd::from_core(&d).is_ok());
}

#[test]
fn the_pixel_budget_counts_each_mask_as_a_whole_canvas() {
    // 4096² のキャンバスに 9 枚のマスク: 1 枚でキャンバス 1 枚ぶん（16 MiB）を数えるので、128 MiB を超えるところで書かず予算の理由を言う
    let mut d = Document::new(4096, 4096).unwrap();
    for n in 0..9 {
        let a = d.add_layer(&format!("m{n}")).unwrap();
        d.add_layer_mask(a).unwrap();
    }
    let err = Psd::from_core(&d).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
    assert!(err.to_string().contains("予算"), "{err}");
}

#[test]
fn undo_of_a_core_edit_after_import_does_not_reach_the_import() {
    let psd = Psd {
        width: 2,
        height: 1,
        layers: vec![Layer {
            id: 1,
            name: "a".into(),
            width: 2,
            height: 1,
            pixels_rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
            mask: Some(Mask {
                left: 0,
                top: 0,
                width: 1,
                height: 1,
                default_color: 255,
                enabled: true,
                density: 255,
                pixels: vec![0],
            }),
            ..Layer::default()
        }],
        composite_rgba: None,
    };
    let mut core = psd.to_core().unwrap();
    assert!(!core.undo().unwrap(), "取り込みは取り消せる段にならない");
    core.set_layer_opacity(core.layers()[0].id(), 0.5, false)
        .unwrap();
    assert!(core.undo().unwrap());
    assert!(!core.undo().unwrap());
    assert!(core.layers()[0].mask().is_some());
}

#[test]
fn export_blockers_name_every_reason_for_every_layer_and_agree_with_from_core() {
    use yolu_io::psd::{export_blockers, Blocker, Refusal};
    let mut d = new_doc();
    // Color を無効にしたレイヤー・ほかのチャンネルだけが違うレイヤーは、Color の PSD では隠すか Color の値で書くので、断る理由にならない
    let off = raster(&mut d, "off", gradient);
    d.set_channel_enabled(off, Channel::Color, false).unwrap();
    let inv = raster(&mut d, "inverted", soft);
    d.add_layer_mask(inv).unwrap();
    d.set_layer_mask_inverted(inv, true).unwrap();
    d.set_channel_opacity(inv, Channel::Height, Some(0.5), false)
        .unwrap();
    adjust(
        &mut d,
        "between",
        AdjustmentSettings::levels(0.3, 1.0, 1.0, 0.0, 1.0).unwrap(),
    );
    let inner = raster(&mut d, "inner", soft);
    let g = d.group_layers(&[inner], "folder").unwrap();
    d.set_layer_clipping(g, true).unwrap();
    let ok = raster(&mut d, "fine", gradient);
    let _ = ok;
    let all = export_blockers(&d);
    let reasons: Vec<(&str, &Refusal)> =
        all.iter().map(|b| (b.layer.as_str(), &b.refusal)).collect();
    assert_eq!(
        reasons,
        [
            ("inverted", &Refusal::InvertedMask),
            ("between", &Refusal::LevelsBetweenSteps),
            ("folder", &Refusal::ClippedGroup),
        ]
    );
    // from_core は上のレイヤーから見て初めの理由で断る（「fine」は書ける。次の「folder」）
    let top_first: &Blocker = all.iter().rev().find(|b| b.layer == "folder").unwrap();
    assert_eq!(refusal(&d), top_first.message());
    // 書ける文書は空
    let mut clean = new_doc();
    raster(&mut clean, "a", gradient);
    assert!(export_blockers(&clean).is_empty());
}

#[test]
fn group_dividers_count_against_the_layer_record_budget() {
    // レイヤー 130 枚は上限の 256 に収まるが、グループは区切りの記録も要るので 260 件になる。書かず予算の理由を言う
    let mut d = new_doc();
    for n in 0..130 {
        d.add_group(&format!("g{n}"), None).unwrap();
    }
    let err = Psd::from_core(&d).unwrap_err();
    assert!(matches!(err, yolu_io::Error::Budget(_)), "{err}");
    assert!(err.to_string().contains("記録"), "{err}");
    // 127 個ならちょうど 254 件で収まり、往復する
    let mut d = new_doc();
    for n in 0..127 {
        d.add_group(&format!("g{n}"), None).unwrap();
    }
    let (back, _) = round_trip(&d);
    assert_eq!(back.layers().len(), 127);
}
