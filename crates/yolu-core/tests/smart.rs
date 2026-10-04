use yolu_core::smart::{SmartKind, SmartMaterial, SmartPlacement, SmartResampling};
use yolu_core::{
    BrushSettings, Channel, ChannelBlend, ChannelInfo, ChannelKind, ColorSpace, CoreError,
    Document, LayerId, Rgba8,
};
/// 文書の変わったかどうかの目安: 層の数・画素のバイト数・版・Undo の有無。
fn state(d: &Document) -> (usize, u64, u64, bool) {
    (
        d.layers().len(),
        d.allocated_bytes(),
        d.revision(),
        d.can_undo(),
    )
}
fn user(name: &str, kind: ChannelKind) -> ChannelInfo {
    ChannelInfo {
        name: name.into(),
        kind,
        color_space: ColorSpace::Linear,
        default: Rgba8::TRANSPARENT,
    }
}
/// 画素を持つ層を `count` 枚。画素は層ごとに違う（再標本化の面が層ごとに数えられる）。
fn painted(width: u32, height: u32, tile: u32, count: u32) -> (Document, Vec<LayerId>) {
    let mut d = Document::with_tile_size(width, height, tile).unwrap();
    let ids: Vec<_> = (0..count)
        .map(|i| {
            let l = d.add_layer(&format!("層{i}")).unwrap();
            for y in 0..height {
                for x in 0..width {
                    let p = Rgba8::new((x * 31 + i * 7) as u8, (y * 17) as u8, (x + y) as u8, 255);
                    d.set_pixel(l, x, y, p).unwrap();
                }
            }
            l
        })
        .collect();
    (d, ids)
}
#[test]
fn capture_place_channels_and_single_undo() {
    let mut source = Document::with_tile_size(4, 3, 2).unwrap();
    let a = source
        .add_fill_layer(
            "色",
            &[
                (Channel::Color, Rgba8::new(20, 30, 40, 255)),
                (Channel::Roughness, Rgba8::new(70, 70, 70, 255)),
            ],
            None,
        )
        .unwrap();
    let b = source.add_layer("画素").unwrap();
    let group = source.group_layers(&[a, b], "組").unwrap();
    let smart = source.capture_smart_material(&[group, a], "素材").unwrap();
    assert_eq!(smart.layers().len(), 3);
    let mut target = Document::with_tile_size(4, 3, 2).unwrap();
    let result = target
        .place_smart_material(
            &smart,
            &SmartPlacement {
                channels: Some(vec![Channel::Color]),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(result.switched_off, vec![Channel::Roughness]);
    assert_eq!(
        target.composite(target.bounds()).unwrap(),
        source.composite(source.bounds()).unwrap()
    );
    assert_ne!(result.layer_id, group);
    target.undo().unwrap();
    assert!(target.layers().is_empty());
    target.redo().unwrap();
    assert_eq!(target.layers().len(), 3);
    assert!(smart.layers()[0].is_channel_enabled(Channel::Roughness));
}
#[test]
fn wraps_multiple_tops_and_inserts_inside_group() {
    let mut d = Document::new(4, 4).unwrap();
    let a = d.add_layer("a").unwrap();
    let b = d.add_layer("b").unwrap();
    let s = d.capture_smart_material(&[b, a], "まとまり").unwrap();
    let g = d.add_group("先", None).unwrap();
    let r = d
        .place_smart_material(
            &s,
            &SmartPlacement {
                parent: Some(g),
                position: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(r.layers.len(), 3);
    assert_eq!(d.layer(r.layer_id).unwrap().name(), "まとまり");
    d.validate_structure().unwrap();
    d.undo().unwrap();
    assert_eq!(d.layers().len(), 3);
}
#[test]
fn mask_replace_undo_and_budget_are_atomic() {
    let mut d = Document::with_tile_size(4, 4, 2).unwrap();
    let a = d.add_layer("a").unwrap();
    d.add_layer_mask(a).unwrap();
    d.set_mask_pixel(a, 0, 0, 123).unwrap();
    let mask = d.capture_smart_mask(a, "マスク").unwrap();
    d.set_mask_pixel(a, 0, 0, 240).unwrap();
    assert!(d.apply_smart_mask(&mask, a, None).unwrap().replaced_mask);
    assert_eq!(
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(0, 0)
            .unwrap()
            .a,
        123
    );
    d.undo().unwrap();
    assert_eq!(
        d.layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(0, 0)
            .unwrap()
            .a,
        240
    );
    let mut target = Document::with_tile_size(4, 4, 2).unwrap();
    let b = target.add_layer("b").unwrap();
    target.set_source_budget_bytes(0).unwrap();
    assert!(target.apply_smart_mask(&mask, b, None).is_err());
    assert!(target.layer(b).unwrap().mask().is_none());
    assert!(target
        .place_smart_material(&mask, &Default::default())
        .is_err());
}
#[test]
fn names_and_empty_capture_are_rejected() {
    let mut d = Document::new(1, 1).unwrap();
    let a = d.add_layer("a").unwrap();
    for name in ["", " ", "a\n", &"😀".repeat(129)] {
        assert!(d.capture_smart_material(&[a], name).is_err());
    }
    assert!(d.capture_smart_material(&[], "素材").is_err());
    assert!(d.capture_smart_mask(a, "マスク").is_err());
}
#[test]
fn resizing_and_retiling_preserve_uniform_transparent_rgb() {
    let mut d = Document::with_tile_size(2, 2, 2).unwrap();
    let a = d.add_layer("a").unwrap();
    for y in 0..2 {
        for x in 0..2 {
            d.set_pixel(a, x, y, Rgba8::new(42, 50, 60, 0)).unwrap();
        }
    }
    let s = d.capture_smart_material(&[a], "透明").unwrap();
    for how in [
        SmartResampling::Nearest,
        SmartResampling::Bilinear,
        SmartResampling::Area,
    ] {
        let mut t = Document::with_tile_size(5, 3, 2).unwrap();
        let r = t
            .place_smart_material(
                &s,
                &SmartPlacement {
                    resampling: Some(how),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(r.resampled);
        let surface = t
            .layer(r.layer_id)
            .unwrap()
            .surface(Channel::Color)
            .unwrap();
        for y in 0..3 {
            for x in 0..5 {
                assert_eq!(surface.pixel(x, y).unwrap(), Rgba8::new(42, 50, 60, 0));
            }
        }
    }
}

// ───────── ユーザーチャンネル ─────────

/// 無効にしたユーザーチャンネルにだけ内容（面・塗りつぶしの値・チャンネルごとの合成）がある素材。
fn disabled_user_content(what: &str) -> (SmartMaterial, Channel) {
    let mut d = Document::with_tile_size(4, 4, 2).unwrap();
    let ch = d.add_channel(user("ユーザー", ChannelKind::Color)).unwrap();
    let layer = match what {
        "surface" => {
            let l = d.add_layer("画素").unwrap();
            d.set_channel_pixel(l, ch, 1, 1, Rgba8::new(9, 8, 7, 255))
                .unwrap();
            l
        }
        "fill" => d
            .add_fill_layer("塗り", &[(ch, Rgba8::new(1, 2, 3, 255))], None)
            .unwrap(),
        _ => {
            let l = d.add_layer("合成").unwrap();
            d.set_channel_blend(l, ch, ChannelBlend::new(None, Some(0.5)), false)
                .unwrap();
            l
        }
    };
    d.set_channel_enabled(layer, ch, false).unwrap();
    assert!(!d.layer(layer).unwrap().is_channel_enabled(ch));
    (d.capture_smart_material(&[layer], "素材").unwrap(), ch)
}
#[test]
fn disabled_user_channel_content_is_refused_without_the_same_channel() {
    for what in ["surface", "fill", "blend"] {
        let (material, ch) = disabled_user_content(what);
        // チャンネルの無い文書、番号が違う文書、情報が違う文書は断り、何も変えない
        let mut none = Document::with_tile_size(4, 4, 2).unwrap();
        let mut shifted = Document::with_tile_size(4, 4, 2).unwrap();
        shifted.add_channel(user("他", ChannelKind::Color)).unwrap();
        shifted
            .add_channel(user("ユーザー", ChannelKind::Color))
            .unwrap();
        let mut other = Document::with_tile_size(4, 4, 2).unwrap();
        other
            .add_channel(user("ユーザー", ChannelKind::Scalar))
            .unwrap();
        for (label, target) in [
            ("無し", &mut none),
            ("番号", &mut shifted),
            ("情報", &mut other),
        ] {
            let before = state(target);
            let result = target.place_smart_material(&material, &SmartPlacement::default());
            assert!(
                matches!(result, Err(CoreError::Unsupported(_))),
                "{what}/{label}"
            );
            assert_eq!(state(target), before, "{what}/{label}");
        }
        // 同じ番号に同じ情報があれば置け、無効のまま内容を保つ
        let mut same = Document::with_tile_size(4, 4, 2).unwrap();
        assert_eq!(
            same.add_channel(user("ユーザー", ChannelKind::Color))
                .unwrap(),
            ch
        );
        let placed = same
            .place_smart_material(&material, &SmartPlacement::default())
            .unwrap();
        let layer = same.layer(placed.layer_id).unwrap();
        assert!(!layer.is_channel_enabled(ch), "{what}");
        match what {
            "surface" => assert_eq!(
                layer.surface(ch).unwrap().pixel(1, 1).unwrap(),
                Rgba8::new(9, 8, 7, 255)
            ),
            "fill" => assert_eq!(layer.fill_value(ch), Some(Rgba8::new(1, 2, 3, 255))),
            _ => assert_eq!(layer.channel_blend(ch).opacity, Some(0.5)),
        }
        same.undo().unwrap();
        assert!(same.layers().is_empty());
    }
}
#[test]
fn unused_user_channel_definitions_are_not_carried_into_a_fragment() {
    let mut d = Document::with_tile_size(4, 4, 2).unwrap();
    let ch = d.add_channel(user("ユーザー", ChannelKind::Color)).unwrap();
    d.add_channel(user("どの層も使わない", ChannelKind::Scalar))
        .unwrap();
    let used = d.add_layer("使う").unwrap();
    d.set_channel_pixel(used, ch, 0, 0, Rgba8::new(5, 5, 5, 255))
        .unwrap();
    let unused = d.add_layer("使わない").unwrap();
    d.add_layer_mask(unused).unwrap();
    let user_channels = |m: &SmartMaterial| {
        m.fragment_document()
            .unwrap()
            .channels()
            .into_iter()
            .filter(|c| !c.is_standard())
            .collect::<Vec<_>>()
    };
    // 使っている層を含めるときだけ、その定義を持つ（使っていない定義は、どちらでも持たない）
    assert_eq!(
        user_channels(&d.capture_smart_material(&[used], "a").unwrap()),
        vec![ch]
    );
    let without = d.capture_smart_material(&[unused], "b").unwrap();
    assert!(user_channels(&without).is_empty());
    assert!(user_channels(&d.capture_smart_mask(unused, "c").unwrap()).is_empty());
    // 定義の無い文書へもそのまま置ける
    let mut plain = Document::with_tile_size(4, 4, 2).unwrap();
    plain
        .place_smart_material(&without, &SmartPlacement::default())
        .unwrap();
}

// ───────── 予算 ─────────

#[test]
fn place_budget_is_exact_and_a_refusal_changes_nothing() {
    let (source, ids) = painted(4, 4, 2, 2);
    let material = source.capture_smart_material(&ids, "素材").unwrap();
    let need = material.pixel_bytes();
    assert!(need > 0);
    // 同じ寸法: 予算ちょうどで置け、1 バイト足りなければ断る。既存の画素も予算に数える
    for existing in [0u32, 1] {
        let mut ok = Document::with_tile_size(4, 4, 2).unwrap();
        let mut short = Document::with_tile_size(4, 4, 2).unwrap();
        for d in [&mut ok, &mut short] {
            for i in 0..existing {
                let l = d.add_layer("既存").unwrap();
                d.set_pixel(l, 0, 0, Rgba8::new(1, 2, 3, 200 + i as u8))
                    .unwrap();
            }
        }
        let held = ok.allocated_bytes();
        ok.set_source_budget_bytes(held + need).unwrap();
        short.set_source_budget_bytes(held + need - 1).unwrap();
        let before = state(&short);
        assert!(matches!(
            short.place_smart_material(&material, &SmartPlacement::default()),
            Err(CoreError::SourceBudgetExceeded)
        ));
        assert_eq!(state(&short), before);
        ok.place_smart_material(&material, &SmartPlacement::default())
            .unwrap();
        assert_eq!(ok.allocated_bytes(), held + need);
    }
    // 寸法が違う（再標本化する）: 置いた結果の大きさでちょうど
    let mut probe = Document::with_tile_size(8, 6, 4).unwrap();
    probe
        .place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
    let resampled = probe.allocated_bytes();
    let mut exact = Document::with_tile_size(8, 6, 4).unwrap();
    exact.set_source_budget_bytes(resampled).unwrap();
    exact
        .place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
    assert_eq!(exact.allocated_bytes(), resampled);
    // 1 バイト足りない（最後の面の途中で超える）と、層を 1 つも入れず、Undo の段も作らない
    let mut short = Document::with_tile_size(8, 6, 4).unwrap();
    short.set_source_budget_bytes(resampled - 1).unwrap();
    let before = state(&short);
    assert!(matches!(
        short.place_smart_material(&material, &SmartPlacement::default()),
        Err(CoreError::SourceBudgetExceeded)
    ));
    assert_eq!(state(&short), before);
    assert!(!short.can_undo());
    // 最初の層は入るが 2 つ目で超える予算でも、途中までの層を残さない
    let first_layer = resampled / 2;
    let mut middle = Document::with_tile_size(8, 6, 4).unwrap();
    middle.set_source_budget_bytes(first_layer).unwrap();
    let before = state(&middle);
    assert!(middle
        .place_smart_material(&material, &SmartPlacement::default())
        .is_err());
    assert_eq!(state(&middle), before);
}
#[test]
fn smart_mask_budget_counts_only_the_growth_over_the_mask_it_replaces() {
    let mut source = Document::with_tile_size(4, 4, 2).unwrap();
    let l = source.add_layer("元").unwrap();
    source.add_layer_mask(l).unwrap();
    for (x, y) in [(0, 0), (3, 3), (0, 3)] {
        source.set_mask_pixel(l, x, y, 100).unwrap();
    }
    let mask = source.capture_smart_mask(l, "マスク").unwrap();
    let need = mask.pixel_bytes();
    assert!(need > 0);
    let mut target = Document::with_tile_size(4, 4, 2).unwrap();
    let a = target.add_layer("先").unwrap();
    // 今のマスクを置き換える分は空く: 予算 = 今の画素 − 今のマスク + 新しいマスク でちょうど
    target.add_layer_mask(a).unwrap();
    target.set_mask_pixel(a, 1, 1, 250).unwrap();
    let old = target
        .layer(a)
        .unwrap()
        .mask()
        .unwrap()
        .surface()
        .allocated_bytes();
    let rest = target.allocated_bytes() - old;
    target.set_source_budget_bytes(rest + need - 1).unwrap();
    let before = state(&target);
    assert!(matches!(
        target.apply_smart_mask(&mask, a, None),
        Err(CoreError::SourceBudgetExceeded)
    ));
    assert_eq!(state(&target), before);
    assert_eq!(
        target
            .layer(a)
            .unwrap()
            .mask()
            .unwrap()
            .surface()
            .pixel(1, 1)
            .unwrap()
            .a,
        250
    );
    target.set_source_budget_bytes(rest + need).unwrap();
    assert!(
        target
            .apply_smart_mask(&mask, a, None)
            .unwrap()
            .replaced_mask
    );
    assert_eq!(target.allocated_bytes(), rest + need);
}

// ───────── 拒否と境界 ─────────

#[test]
fn layer_limit_counts_the_wrapping_group_and_refuses_before_changing_anything() {
    let mut one = Document::with_tile_size(2, 2, 2).unwrap();
    let a = one.add_layer("a").unwrap();
    let single = one.capture_smart_material(&[a], "1層").unwrap();
    let mut pair = Document::with_tile_size(2, 2, 2).unwrap();
    let ids = [pair.add_layer("a").unwrap(), pair.add_layer("b").unwrap()];
    let wrapped = pair.capture_smart_material(&ids, "2層").unwrap();
    let fill = |d: &mut Document, count: usize| {
        while d.layers().len() < count {
            d.add_layer("空").unwrap();
        }
    };
    // 1 層は 2047 層の文書に入る（ちょうど 2048）。もう 1 層は断る
    let mut d = Document::with_tile_size(2, 2, 2).unwrap();
    fill(&mut d, 2047);
    d.place_smart_material(&single, &SmartPlacement::default())
        .unwrap();
    assert_eq!(d.layers().len(), 2048);
    let before = state(&d);
    assert!(d
        .place_smart_material(&single, &SmartPlacement::default())
        .is_err());
    assert_eq!(state(&d), before);
    // 最上位が 2 つなら包むグループが 1 つ増える: 2 + 1 で、2045 層ならちょうど、2046 層なら断る
    let mut d = Document::with_tile_size(2, 2, 2).unwrap();
    fill(&mut d, 2045);
    d.place_smart_material(&wrapped, &SmartPlacement::default())
        .unwrap();
    assert_eq!(d.layers().len(), 2048);
    let mut d = Document::with_tile_size(2, 2, 2).unwrap();
    fill(&mut d, 2046);
    let before = state(&d);
    assert!(matches!(
        d.place_smart_material(&wrapped, &SmartPlacement::default()),
        Err(CoreError::InvalidArgument(_))
    ));
    assert_eq!(state(&d), before);
}
#[test]
fn parent_must_be_an_existing_group_and_a_far_position_goes_on_top() {
    let mut d = Document::with_tile_size(2, 2, 2).unwrap();
    let a = d.add_layer("a").unwrap();
    let material = d.capture_smart_material(&[a], "素材").unwrap();
    let inside = d.add_layer("中").unwrap();
    let group = d.group_layers(&[inside], "組").unwrap();
    let before = state(&d);
    // グループでない層・無い層は親にできない
    assert!(matches!(
        d.place_smart_material(
            &material,
            &SmartPlacement {
                parent: Some(a),
                ..Default::default()
            }
        ),
        Err(CoreError::InvalidArgument(_))
    ));
    assert!(matches!(
        d.place_smart_material(
            &material,
            &SmartPlacement {
                parent: Some(LayerId(0xdead)),
                ..Default::default()
            }
        ),
        Err(CoreError::LayerNotFound)
    ));
    assert_eq!(state(&d), before);
    // 兄弟の数を超える位置は、そのグループの一番上
    let placed = d
        .place_smart_material(
            &material,
            &SmartPlacement {
                parent: Some(group),
                position: Some(99),
                ..Default::default()
            },
        )
        .unwrap();
    let order: Vec<_> = d.layers().iter().map(|l| l.id()).collect();
    assert_eq!(order.last().copied(), Some(group));
    assert_eq!(order[order.len() - 2], placed.layer_id);
    assert!(
        order.iter().position(|id| *id == inside)
            < order.iter().position(|id| *id == placed.layer_id)
    );
    assert_eq!(d.layer(placed.layer_id).unwrap().parent(), Some(group));
    d.validate_structure().unwrap();
}
#[test]
fn nothing_is_captured_or_placed_while_a_stroke_is_active() {
    let mut d = Document::with_tile_size(4, 4, 2).unwrap();
    let a = d.add_layer("a").unwrap();
    d.add_layer_mask(a).unwrap();
    let material = d.capture_smart_material(&[a], "素材").unwrap();
    let mask = d.capture_smart_mask(a, "マスク").unwrap();
    let _stroke = d.begin_stroke(a, &BrushSettings::default()).unwrap();
    let before = state(&d);
    assert!(matches!(
        d.place_smart_material(&material, &SmartPlacement::default()),
        Err(CoreError::StrokeActive)
    ));
    assert!(matches!(
        d.apply_smart_mask(&mask, a, None),
        Err(CoreError::StrokeActive)
    ));
    assert!(matches!(
        d.capture_smart_material(&[a], "別"),
        Err(CoreError::StrokeActive)
    ));
    assert!(matches!(
        d.capture_smart_mask(a, "別"),
        Err(CoreError::StrokeActive)
    ));
    assert_eq!(state(&d), before);
}
#[test]
fn a_smart_material_does_not_go_on_a_mask_and_a_smart_mask_not_on_a_layer() {
    let mut d = Document::with_tile_size(4, 4, 2).unwrap();
    let a = d.add_layer("a").unwrap();
    let material = d.capture_smart_material(&[a], "素材").unwrap();
    d.add_layer_mask(a).unwrap();
    let mask = d.capture_smart_mask(a, "マスク").unwrap();
    assert_eq!(material.kind(), SmartKind::Material);
    assert_eq!(mask.kind(), SmartKind::Mask);
    let before = state(&d);
    let on_mask = d.apply_smart_mask(&material, a, None).unwrap_err();
    assert!(matches!(on_mask, CoreError::InvalidArgument(_)));
    assert!(
        on_mask.to_string().contains("マスクに置けません"),
        "{on_mask}"
    );
    let on_layer = d
        .place_smart_material(&mask, &SmartPlacement::default())
        .unwrap_err();
    assert!(matches!(on_layer, CoreError::InvalidArgument(_)));
    assert!(
        on_layer.to_string().contains("層に置けません"),
        "{on_layer}"
    );
    assert_eq!(state(&d), before);
}
