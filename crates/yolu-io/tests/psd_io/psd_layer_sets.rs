//! PSD の書き出しのレイヤーセット: 2 つ以上なら、レイヤーセットごとに一番上の段のグループ（名前はレイヤーセットの名前、空なら `Set <番号>`）にする。
//! 表示するのは書き出すチャンネルを出力しているレイヤーセットのグループだけで、読み戻した合成は最終の結果と同じ。1 つなら今と同じ PSD。
use yolu_core::{Channel, Document, LayerId, Rgba8};
use yolu_io::psd::{self, ExportControl, ExportMode, ExportOptions, Layer, LayerKind, Limits};

fn paint(d: &mut Document, id: LayerId, c: Rgba8, x0: u32, x1: u32) {
    for y in 0..d.height() {
        for x in x0..x1 {
            d.set_pixel(id, x, y, c).unwrap();
        }
    }
}

fn export(d: &Document, channel: Channel) -> psd::Document {
    let ctl = ExportControl::default();
    let plan = psd::plan_export(d, &ExportOptions::new(channel, ExportMode::Bake), &ctl).unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    plan.build(d, &ctl).unwrap().document
}

fn names(layers: &[Layer]) -> Vec<(String, bool)> {
    layers.iter().map(|l| (l.name.clone(), l.visible)).collect()
}

fn children(l: &Layer) -> &[Layer] {
    match &l.kind {
        LayerKind::Group { children, .. } => children,
        other => panic!("グループのはず: {other:?}"),
    }
}

#[test]
fn each_layer_set_becomes_a_top_level_group_and_only_the_output_is_shown() {
    let mut d = Document::with_tile_size(32, 16, 8).unwrap();
    let a = d.add_layer("下地").unwrap();
    paint(&mut d, a, Rgba8::new(200, 30, 30, 255), 0, 32);
    let single = export(&d, Channel::Color);
    let first = d.layer_sets()[0].id;
    let named = d.add_layer_set("影").unwrap();
    let b = d.add_layer("影のレイヤー").unwrap();
    paint(&mut d, b, Rgba8::new(10, 10, 10, 160), 8, 24);
    d.move_layers_to_set(&[b], named).unwrap();
    d.set_channel_output(Channel::Color, Some(named)).unwrap();
    let out = export(&d, Channel::Color);
    assert_eq!(out.layers.len(), 2);
    let mut top = names(&out.layers);
    top.sort();
    assert_eq!(
        top,
        [("Set 1".to_owned(), false), ("影".to_owned(), true)],
        "名前の空のレイヤーセットは並びの番号"
    );
    for g in &out.layers {
        let inner: Vec<_> = children(g).iter().map(|l| l.name.clone()).collect();
        let want = if g.name == "影" {
            "影のレイヤー"
        } else {
            "下地"
        };
        assert_eq!(inner, [want]);
    }
    // 読み戻した合成は、書き出したチャンネルの最終の結果
    let bytes = psd::write(&out, &Limits::default()).unwrap();
    let back = psd::read(&bytes, &Limits::default())
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(
        back.composite_channel(Channel::Color, back.bounds())
            .unwrap(),
        d.composite_channel(Channel::Color, d.bounds()).unwrap()
    );
    // 書き出しは文書を変えない
    assert_eq!(d.layer_sets().len(), 2);
    assert_eq!(d.layer_set_of(a).unwrap(), first);
    // レイヤーセットを 1 つに戻せば、今と同じ PSD
    d.remove_layer_set(named).unwrap();
    assert_eq!(export(&d, Channel::Color), single);
}

#[test]
fn an_anchor_in_another_layer_set_is_still_not_read_after_export() {
    use yolu_core::generator::{self, anchor::ReadMode, Settings};
    use yolu_core::{AnchorPlacement, EffectSettings, FilterSpec, FilterTarget};
    let mut d = Document::with_tile_size(32, 16, 8).unwrap();
    let a = d.add_layer("下").unwrap();
    paint(&mut d, a, Rgba8::new(200, 30, 30, 255), 0, 32);
    let b = d.add_layer("上").unwrap();
    paint(&mut d, b, Rgba8::new(10, 40, 230, 255), 0, 32);
    let anchor = d.add_anchor(a, AnchorPlacement::Layer, None, None).unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let fid = d
        .add_filter(
            b,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
        )
        .unwrap();
    d.set_generator_anchor(b, fid, Some(anchor), Channel::Color, ReadMode::Value, false)
        .unwrap();
    // a だけを別のレイヤーセットへ移す（b から見て、その Anchor は別のレイヤーセットのもの）
    let other = d.add_layer_set("x").unwrap();
    d.move_layers_to_set(&[a], other).unwrap();
    let want = d.composite_channel(Channel::Color, d.bounds()).unwrap();
    let out = export(&d, Channel::Color);
    let bytes = psd::write(&out, &Limits::default()).unwrap();
    let back = psd::read(&bytes, &Limits::default())
        .unwrap()
        .to_core()
        .unwrap();
    assert_eq!(
        back.composite_channel(Channel::Color, back.bounds())
            .unwrap(),
        want
    );
}
