//! レイヤーの操作と効果・パスのつなぎ目: 複製・削除・変形・結合・大きさの変更が、効果の ID・半径・評価した出力・パスを C# と同じに扱う。
//! どれも 1 回の Undo で戻り、断ったら何も変えない。人工データだけ。
#![allow(clippy::chunks_exact_to_as_chunks)]
use crate::attach_support;
use crate::seam_support;
use attach_support::*;
use seam_support::*;
use std::collections::HashSet;
use yolu_core::fill_image::{Projection, ProjectionMode};
use yolu_core::generator::{self, anchor::ReadMode, Settings};
use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};
use yolu_core::paths::{
    fingerprint, render_canvas, render_surface, Options, PathPoint, SurfacePath,
};
use yolu_core::{
    Affine2D, AnchorPlacement, CanvasResampling, Channel, CoreError, Document, EffectSettings,
    FilterSpec, FilterTarget, LayerId, LayerPath, Resampling, Rgba8,
};

fn colour(doc: &Document) -> Vec<u8> {
    whole(doc, Channel::Color)
}

/// 2 つの合成の、バイトごとの差の最大。
fn max_difference(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// 合成が同じ見た目か（アルファが 0 の画素は、RGB が違っても同じ。結合したレイヤーは完全に透明な画素の色を持たない）。
fn assert_same_look(a: &[u8], b: &[u8], what: &str) {
    assert_eq!(a.len(), b.len());
    for (i, (p, q)) in a.chunks_exact(4).zip(b.chunks_exact(4)).enumerate() {
        if p[3] == 0 && q[3] == 0 {
            continue;
        }
        assert_eq!(
            p,
            q,
            "{what}: 画素 ({}, {})",
            i % W as usize,
            i / W as usize
        );
    }
}

/// 評価のキャッシュを捨てて作り直した合成と、今の合成が同じか（古い出力を返していないか）。
fn assert_cache_is_honest(doc: &Document, what: &str) {
    let now: Vec<_> = Channel::ALL.iter().map(|c| whole(doc, *c)).collect();
    doc.release_effect_cache();
    let fresh: Vec<_> = Channel::ALL.iter().map(|c| whole(doc, *c)).collect();
    assert!(
        now == fresh,
        "{what}: キャッシュの出力が作り直した出力と違う"
    );
}

fn all_ids(doc: &Document) -> (Vec<u128>, Vec<u128>, Vec<u128>) {
    let filters = doc
        .layers()
        .iter()
        .flat_map(|l| {
            l.filters().iter().map(|e| e.id().0).chain(
                l.mask()
                    .into_iter()
                    .flat_map(|m| m.filters().iter().map(|e| e.id().0)),
            )
        })
        .collect();
    let anchors = doc.anchors().iter().map(|a| a.anchor.id().0).collect();
    let paths = doc
        .layers()
        .iter()
        .filter_map(|l| l.path().map(LayerPath::id))
        .collect();
    (filters, anchors, paths)
}

fn unique(v: &[u128]) -> bool {
    v.iter().collect::<HashSet<_>>().len() == v.len()
}

// ───────── 複製・削除 ─────────

/// 複数のレイヤーの複製は 1 回の Undo で、写しの段・Anchor・パスは新しい ID を持つ（文書の中で重ならない）。Undo で写しが消える。
#[test]
fn duplicating_layers_renews_every_effect_id_in_one_step() {
    let mut r = rig();
    let before = all_ids(&r.doc);
    assert!(!before.0.is_empty() && !before.1.is_empty() && !before.2.is_empty());
    let layers = r.doc.layers().len();
    let steps = r.doc.undo_count();
    let copies = r
        .doc
        .duplicate_layers(&[r.base, r.top, r.path_layer, r.fill])
        .unwrap();
    assert_eq!(copies.len(), 4);
    assert_eq!(r.doc.undo_count(), steps + 1);
    let after = all_ids(&r.doc);
    assert!(
        unique(&after.0) && unique(&after.1) && unique(&after.2),
        "ID は文書の中で重ならない"
    );
    assert_eq!(
        after.0.len(),
        before.0.len() * 2 - 1 + 1 - 1,
        "段が倍（マスクの段・レイヤーの段とも）"
    );
    assert_eq!(
        after.1.len(),
        before.1.len() + 1,
        "写したレイヤーの Anchor も別の ID で持つ"
    );
    assert_eq!(after.2.len(), before.2.len() * 2, "パスも別の ID");
    // 写しは元の効果を持つ（設定は同じ・ID だけ違う）
    let original = r.doc.layer(r.base).unwrap().filters()[0].settings().clone();
    let copy_of_base = r.doc.layer(copies[0]).unwrap();
    assert_eq!(copy_of_base.filters()[0].settings(), &original);
    assert_ne!(copy_of_base.filters()[0].id(), r.blur);
    // 取り消すと写しは消え、ID の集まりも元に戻る
    r.doc.undo().unwrap();
    assert_eq!(r.doc.layers().len(), layers);
    assert_eq!(all_ids(&r.doc), before);
    r.doc.redo().unwrap();
    assert_eq!(all_ids(&r.doc), after);
    assert_cache_is_honest(&r.doc, "複製のあと");
}

/// レイヤーの削除（複数）: 効果のレイヤーを消しても、ほかのレイヤーの効果と ID はそのまま。消した Anchor を読む段は、理由を出して入力のまま通し、
/// Undo で戻る。
#[test]
fn removing_layers_with_effects_leaves_readers_passing_through_until_undone() {
    let mut r = rig();
    assert!(r.doc.anchor_issues().is_empty());
    let composite = colour(&r.doc);
    let height = whole(&r.doc, Channel::Height);
    r.doc.remove_layers(&[r.base]).unwrap();
    assert_eq!(r.doc.anchor_issues().len(), 1, "読む Anchor が無くなった段");
    r.doc.undo().unwrap();
    assert!(r.doc.anchor_issues().is_empty());
    assert_eq!(colour(&r.doc), composite);
    assert_eq!(whole(&r.doc, Channel::Height), height);
    assert_cache_is_honest(&r.doc, "削除を取り消したあと");
}

// ───────── 変形 ─────────

/// 変形（移動）の後も段は残り、移った画素を読む。評価した出力のキャッシュは古い位置を返さない。Undo・Redo でも同じ。
#[test]
fn transforming_a_filtered_layer_keeps_its_effects_and_never_serves_a_stale_output() {
    let (mut doc, l) = world();
    let (base, mid, top) = (l[0], l[1], l[2]);
    let f = doc
        .add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(3)).channels(&[Channel::Color]),
        )
        .unwrap();
    doc.add_layer_mask(top).unwrap();
    doc.add_filter(
        top,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(2)),
    )
    .unwrap();
    let a = doc
        .add_anchor(base, AnchorPlacement::Layer, None, None)
        .unwrap();
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            mid,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        mid,
        reader,
        Some(a),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc.clear_history().unwrap();
    let before = colour(&doc);
    let before_height = whole(&doc, Channel::Height);
    // 整数の移動（画素をそのまま写す）と、補間する回転
    for t in [
        Affine2D::translation(3., -2.),
        Affine2D::from_parts((20., 14.), (1., 0.), 17., (1.2, 0.8)).unwrap(),
    ] {
        assert!(doc
            .transform_layers(&[base, top], t, Resampling::Bilinear)
            .unwrap());
        assert_eq!(
            doc.layer(base).unwrap().filters()[0].id(),
            f,
            "段はそのまま"
        );
        let after = colour(&doc);
        assert_ne!(after, before);
        assert_cache_is_honest(&doc, "変形のあと");
        // Anchor を読む Height も、移った土台を読んだ値になる（キャッシュの古い値でない）
        assert_ne!(whole(&doc, Channel::Height), before_height);
        doc.undo().unwrap();
        assert_eq!(colour(&doc), before, "Undo で元の合成");
        assert_eq!(whole(&doc, Channel::Height), before_height);
        assert_cache_is_honest(&doc, "変形を取り消したあと");
        doc.redo().unwrap();
        assert_eq!(colour(&doc), after, "Redo で同じ合成");
        doc.undo().unwrap();
    }
}

// ───────── 結合 ─────────

fn painted_pair(doc: &mut Document) -> (LayerId, LayerId) {
    let lower = doc.add_layer("下").unwrap();
    paint(doc, lower, Channel::Color, 11);
    let upper = doc.add_layer("上").unwrap();
    // 上のレイヤーは 1 つのタイルの隅だけ描く（ぼかしが隣のタイルへ届くように）
    for y in 5..8 {
        for x in 5..8 {
            doc.set_channel_pixel(upper, Channel::Color, x, y, Rgba8::new(240, 30, 60, 255))
                .unwrap();
        }
    }
    (lower, upper)
}

fn plain_doc() -> Document {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    doc.set_effect_inputs(inputs(0)).unwrap();
    doc.set_filter_block_pixels(16).unwrap();
    doc
}

/// 結合は、上のレイヤーのフィルターを画素へ焼く（結果は効果を持たない）。ぼかしが元の画素の無いタイルへ届く分も焼き、合成は変わらない
/// （許容差の内）。Undo で段が戻り、評価の出力も戻る。
#[test]
fn merge_down_bakes_the_upper_layers_filters_including_the_halo_tiles() {
    let mut doc = plain_doc();
    let (_, upper) = painted_pair(&mut doc);
    doc.add_filter(
        upper,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(6)).channels(&[Channel::Color]),
    )
    .unwrap();
    doc.clear_history().unwrap();
    let before = colour(&doc);
    // 上のレイヤーの元の画素は 1 タイルの中だけ。ぼかしの出力は隣のタイルにも出ている
    assert_eq!(
        doc.layer(upper)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .tile_count(),
        1
    );
    assert!(Rgba8::from_slice(&before[(6 * W as usize + 10) * 4..][..4]).a > 0);
    let report = doc.merge_down(upper, 2).unwrap();
    assert_eq!(report.notes & 1, 1, "フィルターを焼いた印");
    assert!(report.max_visible_difference <= 2);
    assert!(max_difference(&before, &colour(&doc)) <= 2);
    let result = doc.layer(report.result_id).unwrap();
    assert!(result.filters().is_empty(), "焼いたあとは段が無い");
    assert!(
        result.surface(Channel::Color).unwrap().tile_count() > 1,
        "ぼかしが届いた隣のタイルも画素になっている"
    );
    assert_eq!(doc.layers().len(), 1);
    assert_cache_is_honest(&doc, "結合のあと");
    // Undo で段が戻り、元の合成へ。Redo でまた焼いた結果へ
    let merged = colour(&doc);
    doc.undo().unwrap();
    assert_eq!(doc.layers().len(), 2);
    assert_eq!(doc.layer(upper).unwrap().filters().len(), 1);
    assert_eq!(colour(&doc), before);
    doc.redo().unwrap();
    assert_eq!(colour(&doc), merged);
}

/// 下のレイヤーが 1 枚目で、ふつうのレイヤーでない（不透明度 50%）と、分離の結合になり、下のレイヤーのマスクも焼く: 厳密に同じ見た目（許容差 0）。
/// 結合したレイヤーは 100% の Normal で、マスクを持たない。
#[test]
fn an_isolated_merge_bakes_the_lower_mask_filters_and_is_exact() {
    let mut doc = plain_doc();
    let (lower, upper) = painted_pair(&mut doc);
    doc.add_layer_mask(lower).unwrap();
    for y in 0..H {
        for x in 0..W {
            if (x + y) % 3 == 0 {
                doc.set_mask_pixel(lower, x, y, 140).unwrap();
            }
        }
    }
    doc.add_filter(
        lower,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(2)),
    )
    .unwrap();
    doc.add_filter(
        upper,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::levels(0.1, 0.9, 1.2, 0.0, 1.0))
            .channels(&[Channel::Color]),
    )
    .unwrap();
    doc.set_layer_opacity(lower, 0.5, false).unwrap();
    doc.clear_history().unwrap();
    let before = colour(&doc);
    let report = doc.merge_down(upper, 0).unwrap();
    assert_eq!(report.method, yolu_core::MergeMethod::Isolated);
    assert_eq!(report.notes & 1, 1);
    assert!(report.exact(), "{report:?}");
    assert_same_look(&before, &colour(&doc), "分離の結合");
    let result = doc.layer(report.result_id).unwrap();
    assert!(result.mask().is_none() && result.filters().is_empty());
    assert_eq!(result.opacity(), 1.0);
}

/// 結合のあとも下のレイヤーのマスクを残すとき（分離でない）、マスクのフィルターは焼かずに結果へ写る（効果のまま）。
#[test]
fn a_kept_mask_keeps_its_filters() {
    let mut doc = plain_doc();
    let (lower, upper) = painted_pair(&mut doc);
    doc.add_layer_mask(lower).unwrap();
    // マスクが隠すのは上のレイヤーの画素（左下の隅）から遠い所だけ（結合したレイヤーはマスクの下へ入るので、近いと見た目が変わる）
    for y in 0..H {
        for x in 28..W {
            doc.set_mask_pixel(lower, x, y, 200).unwrap();
        }
    }
    doc.add_filter(
        lower,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(2)),
    )
    .unwrap();
    let below = doc.add_layer("一番下").unwrap();
    doc.set_pixel(below, 3, 3, Rgba8::new(1, 2, 3, 255))
        .unwrap();
    // 一番下を下へ送って、下のレイヤー（lower）の下に何かが見えるようにする（分離の結合にならない）
    doc.move_layers(&[below], None, 0).unwrap();
    doc.clear_history().unwrap();
    let before = colour(&doc);
    let report = doc.merge_down(upper, 2).unwrap();
    assert_ne!(report.method, yolu_core::MergeMethod::Isolated);
    assert_eq!(
        report.notes & 1,
        0,
        "フィルターを焼いていない（マスクの段は残る）"
    );
    let result = doc.layer(report.result_id).unwrap();
    assert_eq!(result.mask().unwrap().filters().len(), 1);
    assert!(max_difference(&before, &colour(&doc)) <= 2);
}

/// 元の画素が無いタイルへ出す効果（形のグラデーションの Generator）も、全部のレイヤーを結合するとき焼かれる（合成は同じ）。
#[test]
fn merge_visible_bakes_a_generator_into_tiles_that_have_no_pixels() {
    let (mut doc, _) = world();
    let layer = doc.add_layer("空のレイヤー").unwrap();
    let mut g = Settings::new(generator::Kind::ShapeGradient);
    g.ramp = Some(generator::Ramp::default());
    g.blend = generator::Blend::Replace;
    doc.add_filter(
        layer,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
    )
    .unwrap();
    assert!(doc.inactive_effects().is_empty());
    doc.clear_history().unwrap();
    let before: Vec<_> = Channel::ALL.iter().map(|c| whole(&doc, *c)).collect();
    let report = doc.merge_visible("全部", 0).unwrap();
    assert_eq!(report.notes & 1, 1);
    let after: Vec<_> = Channel::ALL.iter().map(|c| whole(&doc, *c)).collect();
    for (c, (b, a)) in Channel::ALL.iter().zip(before.iter().zip(&after)) {
        assert!(max_difference(b, a) == 0, "{c:?}");
    }
    assert!(doc.layer(report.result_id).unwrap().filters().is_empty());
    assert_cache_is_honest(&doc, "結合のあと");
}

/// 文書の見え方と履歴の状態（断ったあとに変わっていないことの確認用）: 版・レイヤーの数と ID・履歴の数・全チャンネルの合成。
fn state_of(doc: &Document) -> (u64, Vec<LayerId>, usize, usize, Vec<Vec<u8>>) {
    (
        doc.revision(),
        doc.layers().iter().map(|l| l.id()).collect(),
        doc.undo_count(),
        doc.redo_count(),
        Channel::ALL.iter().map(|c| whole(doc, *c)).collect(),
    )
}

/// 効いていない効果（使えるマップが無い Generator）は焼き込めない: 落とさず断り、何も変えない。断るたびに、版・レイヤー・履歴・合成が
/// 最初のまま（途中で文書を変えてから断る退行を見逃さない）。
#[test]
fn merges_refuse_to_bake_an_inactive_generator_and_change_nothing() {
    let mut doc = Document::with_tile_size(W, H, 8).unwrap();
    // 入力（メッシュマップ）を渡していないので、Position を読む Generator は入力のまま通す
    let (lower, upper) = painted_pair(&mut doc);
    let mut g = Settings::new(generator::Kind::ShapeGradient);
    g.ramp = Some(generator::Ramp::default());
    g.blend = generator::Blend::Replace;
    doc.add_filter(
        upper,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Color]),
    )
    .unwrap();
    assert_eq!(doc.inactive_effects().len(), 1);
    doc.clear_history().unwrap();
    let before = state_of(&doc);
    let refused = |doc: &mut Document,
                   r: &dyn Fn(&mut Document) -> Result<(), CoreError>,
                   what: &str,
                   owner: LayerId| {
        match r(doc) {
            Err(CoreError::InactiveEffect { layer, mask, .. }) => {
                assert_eq!(layer, owner, "{what}");
                assert!(!mask, "{what}");
            }
            other => panic!("{what}: {other:?}"),
        }
        assert!(state_of(doc) == before, "{what}: 断ったのに文書が変わった");
    };
    refused(
        &mut doc,
        &|d| d.merge_down(upper, 255).map(|_| ()),
        "merge_down",
        upper,
    );
    refused(
        &mut doc,
        &|d| d.merge_visible("全部", 255).map(|_| ()),
        "merge_visible",
        upper,
    );
    refused(
        &mut doc,
        &|d| d.merge_layers(&[lower, upper], 255).map(|_| ()),
        "merge_layers",
        upper,
    );
    // 断ったあとも評価した出力は同じ（何も焼かれていない）
    assert_cache_is_honest(&doc, "断ったあと");
    let group = doc.group_layers(&[lower, upper], "g").unwrap();
    doc.clear_history().unwrap();
    let before = state_of(&doc);
    let refused_group = |doc: &mut Document| {
        match doc.merge_group(group, 255) {
            Err(CoreError::InactiveEffect { layer, mask, .. }) => {
                assert_eq!(layer, upper);
                assert!(!mask);
            }
            other => panic!("merge_group: {other:?}"),
        }
        assert!(
            state_of(doc) == before,
            "merge_group: 断ったのに文書が変わった"
        );
    };
    refused_group(&mut doc);
    assert_eq!(doc.undo_count(), 0);
}

/// 下のレイヤーのマスクに効いていない Generator（Anchor を選んでいない）がある結合。分離の結合（下のレイヤーが 1 枚目でふつうのレイヤーでない）は
/// マスクのフィルターも画素へ焼くので、落とさず断って何も変えない。C# にはこの検査が無く黙って落とす（意図して変えた所）。
/// 分離でない結合はマスクが効果ごと結果へ残るので通る（C# と同じ）。
#[test]
fn an_isolated_merge_refuses_an_inactive_generator_in_the_lower_mask() {
    let anchorless = || {
        let mut g = Settings::new(generator::Kind::Anchor);
        g.blend = generator::Blend::Replace;
        EffectSettings::generator(g)
    };
    let make = |isolated: bool| {
        let mut doc = plain_doc();
        let (lower, upper) = painted_pair(&mut doc);
        doc.add_layer_mask(lower).unwrap();
        for y in 0..H {
            for x in 0..W {
                if (x + y) % 3 == 0 {
                    doc.set_mask_pixel(lower, x, y, 140).unwrap();
                }
            }
        }
        doc.add_filter(lower, FilterTarget::Mask, FilterSpec::new(anchorless()))
            .unwrap();
        if isolated {
            doc.set_layer_opacity(lower, 0.5, false).unwrap();
        } else {
            // 下のレイヤーの下に見えるレイヤーがあると、分離の結合にならない
            let below = doc.add_layer("一番下").unwrap();
            doc.set_pixel(below, 3, 3, Rgba8::new(1, 2, 3, 255))
                .unwrap();
            doc.move_layers(&[below], None, 0).unwrap();
        }
        assert_eq!(doc.inactive_effects().len(), 1);
        doc.clear_history().unwrap();
        (doc, lower, upper)
    };
    // 分離の結合: 断る（持ち主は下のレイヤーのマスク）。何も変わらず、Undo の履歴も増えない
    let (mut doc, lower, upper) = make(true);
    let before = state_of(&doc);
    match doc.merge_down(upper, 255) {
        Err(CoreError::InactiveEffect { layer, mask, .. }) => {
            assert_eq!(layer, lower);
            assert!(mask, "マスクの段");
        }
        other => panic!("{other:?}"),
    }
    assert!(state_of(&doc) == before);
    assert_cache_is_honest(&doc, "断ったあと");
    // 分離でない結合（下のレイヤーの下に見えるレイヤーがある）: 通り、マスクの段は結果に残る
    let (mut doc, _, upper) = make(false);
    let report = doc.merge_down(upper, 255).unwrap();
    assert_ne!(report.method, yolu_core::MergeMethod::Isolated);
    assert_eq!(
        doc.layer(report.result_id)
            .unwrap()
            .mask()
            .unwrap()
            .filters()
            .len(),
        1
    );
}

/// 結合したレイヤーは、上のレイヤーの Anchor を引き継ぐ（読む段はそのまま使える）。下のレイヤーの Anchor は無くなる。
#[test]
fn merge_down_moves_the_upper_anchor_to_the_result() {
    let mut doc = plain_doc();
    let (lower, upper) = painted_pair(&mut doc);
    doc.add_filter(
        upper,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(2)).channels(&[Channel::Color]),
    )
    .unwrap();
    let lower_anchor = doc
        .add_anchor(lower, AnchorPlacement::Layer, Some("下"), None)
        .unwrap();
    let upper_anchor = doc
        .add_anchor(upper, AnchorPlacement::Layer, Some("上"), None)
        .unwrap();
    let reader_layer = doc.add_layer("読むレイヤー").unwrap();
    paint(&mut doc, reader_layer, Channel::Color, 21);
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            reader_layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        reader_layer,
        reader,
        Some(upper_anchor),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc.clear_history().unwrap();
    let before = whole(&doc, Channel::Height);
    let report = doc.merge_down(upper, 2).unwrap();
    let anchors = doc.anchors();
    assert_eq!(anchors.len(), 1);
    assert_eq!(
        anchors[0].anchor.id(),
        upper_anchor,
        "上のレイヤーの Anchor が結果に付く"
    );
    assert_eq!(anchors[0].layer, report.result_id);
    assert!(
        doc.find_anchor(lower_anchor).is_none(),
        "下のレイヤーの Anchor は無くなる"
    );
    assert!(doc.anchor_issues().is_empty(), "読む段はそのまま使える");
    assert!(max_difference(&before, &whole(&doc, Channel::Height)) <= 2);
    doc.undo().unwrap();
    assert_eq!(doc.anchors().len(), 2);
    assert_eq!(whole(&doc, Channel::Height), before);
}

/// 読まれている Anchor が結合で無くなると、読む段は入力のまま通すので見た目が変わる。結合の報告はその変化を数え、許容差を超えれば
/// 何も変えずに断る。C# の結合の報告はこの変化を数えそこなう（派生の Anchor のキャッシュが古い。golden の事例は読む段を外してある）
/// ので、ここは Rust だけで確かめる。
#[test]
fn merging_away_a_read_anchor_is_counted_as_a_change() {
    let mut doc = plain_doc();
    let (lower, upper) = painted_pair(&mut doc);
    paint(&mut doc, lower, Channel::Height, 5);
    let anchor = doc
        .add_anchor(lower, AnchorPlacement::Layer, Some("下"), None)
        .unwrap();
    let reader_layer = doc.add_layer("読むレイヤー").unwrap();
    paint(&mut doc, reader_layer, Channel::Height, 21);
    let mut g = Settings::new(generator::Kind::Anchor);
    g.blend = generator::Blend::Replace;
    let reader = doc
        .add_filter(
            reader_layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::generator(g)).channels(&[Channel::Height]),
        )
        .unwrap();
    doc.set_generator_anchor(
        reader_layer,
        reader,
        Some(anchor),
        Channel::Height,
        ReadMode::Value,
        false,
    )
    .unwrap();
    doc.clear_history().unwrap();
    let before = whole(&doc, Channel::Height);
    match doc.merge_down(upper, 0) {
        Err(CoreError::MergeAppearance(r)) => {
            assert!(
                r.changed_by_channel
                    .get(&Channel::Height)
                    .copied()
                    .unwrap_or(0)
                    > 0
            );
            assert!(r.max_visible_difference > 0);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(whole(&doc, Channel::Height), before, "断ったら何も変えない");
    assert_eq!(doc.undo_count(), 0);
    let report = doc.merge_down(upper, 255).unwrap();
    assert!(
        report
            .changed_by_channel
            .get(&Channel::Height)
            .copied()
            .unwrap_or(0)
            > 0
    );
    assert_ne!(
        whole(&doc, Channel::Height),
        before,
        "読む段は入力のまま通す"
    );
    doc.undo().unwrap();
    assert_eq!(whole(&doc, Channel::Height), before);
}

/// レイヤーの組・グループの結合: 子の効果とパスを焼き、グループの Anchor を結果に移す。印はフィルター 1・パス 2。
#[test]
fn merging_a_group_and_chosen_layers_bakes_effects_and_paths() {
    let mut r = rig();
    // グループ: 土台（ぼかし・Anchor）とパスレイヤー。グループにも Anchor
    let group = r
        .doc
        .group_layers(&[r.base, r.path_layer], "グループ")
        .unwrap();
    let group_anchor = r
        .doc
        .add_anchor(group, AnchorPlacement::Layer, Some("グループ"), None)
        .unwrap();
    r.doc.clear_history().unwrap();
    let before = colour(&r.doc);
    // Anchor を読む中のレイヤー（mid）が土台の Anchor を読む。グループの中の Anchor は無くなるので、読む段は入力のまま通す
    let report = r.doc.merge_group(group, 255).unwrap();
    assert_eq!(report.notes & 3, 3, "フィルターを焼き、パスを画素にした");
    let result = r.doc.layer(report.result_id).unwrap();
    assert!(result.filters().is_empty() && result.path().is_none());
    assert_eq!(
        r.doc.find_anchor(group_anchor).unwrap().layer,
        report.result_id,
        "グループの Anchor が結果へ"
    );
    r.doc.undo().unwrap();
    assert_eq!(colour(&r.doc), before);
    assert!(r.doc.layer(r.path_layer).unwrap().path().is_some());
    // レイヤーの組（選んだ 2 レイヤー）の結合
    let report = r.doc.merge_layers(&[r.base, r.path_layer], 255).unwrap();
    assert_eq!(report.notes & 3, 3);
    let result = r.doc.layer(report.result_id).unwrap();
    assert!(result.filters().is_empty() && result.path().is_none());
    r.doc.undo().unwrap();
    assert_eq!(colour(&r.doc), before);
    assert_cache_is_honest(&r.doc, "結合を取り消したあと");
}

/// 結合の前後を比べる準備用の文書は、本物の文書の入力（画像・メッシュマップ）を持つ: 結合しない画像の塗りつぶしが、比べるときも
/// 画像のまま読まれ、結合が見た目を変えたと取り違えない。
#[test]
fn the_comparison_copy_reads_the_documents_inputs() {
    let mut doc = plain_doc();
    let (lower, upper) = painted_pair(&mut doc);
    let fill = doc
        .add_fill_layer(
            "画像",
            &[(Channel::Color, Rgba8::new(255, 255, 255, 255))],
            None,
        )
        .unwrap();
    doc.set_fill_image(fill, Channel::Color, Some(image(0)))
        .unwrap();
    doc.set_layer_opacity(fill, 0.5, false).unwrap();
    doc.set_fill_projection(
        fill,
        Projection {
            mode: ProjectionMode::Planar,
            tiles: [2.0, 2.0],
            ..Default::default()
        },
        false,
    )
    .unwrap();
    doc.clear_history().unwrap();
    let before = colour(&doc);
    // 下 2 レイヤーを結合する（画像のレイヤーは結合に入らず、上に見える）。準備用の文書が入力を持たないと、画像のレイヤーが値に変わって違いが出る
    let report = doc.merge_layers(&[lower, upper], 0).unwrap();
    assert!(report.exact(), "{report:?}");
    assert_eq!(colour(&doc), before);
}

// ───────── 大きさの変更 ─────────

fn blur_radii(doc: &Document, layer: LayerId) -> Vec<u32> {
    doc.layer(layer)
        .unwrap()
        .filters()
        .iter()
        .filter_map(|e| match e.settings() {
            EffectSettings::Filter(yolu_core::filter::Settings::GaussianBlur { radius }) => {
                Some(*radius)
            }
            _ => None,
        })
        .collect()
}

/// 画像のサイズ変更は、ぼかしの半径を倍率に合わせる（C# の Resampled と同じ丸め・上限・下限）。限った分は理由を返す。マスクの段も。
/// Undo で半径が戻り、評価の出力は新しい大きさのものになる。
#[test]
fn resizing_scales_filter_radii_like_csharp_and_says_what_it_limited() {
    let (mut doc, l) = world();
    let (base, top) = (l[0], l[2]);
    let big = |doc: &mut Document, layer: LayerId, radius: u32| {
        doc.add_filter(
            layer,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(radius)).channels(&[Channel::Color]),
        )
        .unwrap();
    };
    big(&mut doc, base, 3);
    big(&mut doc, base, 200);
    big(&mut doc, base, 1);
    doc.add_layer_mask(top).unwrap();
    doc.add_filter(
        top,
        FilterTarget::Mask,
        FilterSpec::new(EffectSettings::blur(5)),
    )
    .unwrap();
    doc.clear_history().unwrap();
    let before = colour(&doc);
    // 2 倍: 3 → 6、200 → 400 → 上限 256、1 → 2、マスクの 5 → 10
    let report = doc
        .resize_image(80, 56, CanvasResampling::Bilinear)
        .unwrap();
    assert_eq!(blur_radii(&doc, base), vec![6, 256, 2]);
    let mask_radii: Vec<_> = doc
        .layer(top)
        .unwrap()
        .mask()
        .unwrap()
        .filters()
        .iter()
        .map(|e| e.settings().clone())
        .collect();
    assert_eq!(mask_radii, vec![EffectSettings::blur(10)]);
    assert_eq!(
        report.notes.len(),
        1,
        "上限へ限ったのは 200 → 256 だけ: {:?}",
        report.notes
    );
    assert!(report.notes[0].contains("256"), "{:?}", report.notes);
    assert_cache_is_honest(&doc, "拡大のあと");
    doc.undo().unwrap();
    assert_eq!(blur_radii(&doc, base), vec![3, 200, 1]);
    assert_eq!(colour(&doc), before);
    assert_cache_is_honest(&doc, "拡大を取り消したあと");
    // 1/4 に縮める: 3 → 0.75 → 1（下限へ）、200 → 50、1 → 0.25 → 1（下限）
    let report = doc.resize_image(10, 7, CanvasResampling::Area).unwrap();
    assert_eq!(blur_radii(&doc, base), vec![1, 50, 1]);
    // 3 → 0.75 → 1 は範囲の中（限っていない）。1 → 0.25 → 0 だけが下限の 1 へ限られる
    assert_eq!(report.notes.len(), 1, "{:?}", report.notes);
    assert!(report.notes[0].contains("最小"), "{:?}", report.notes);
}

/// 新しい大きさで段の到達半径の合計が上限（512）を超えるなら、何も変えずに断る。
#[test]
fn resizing_refuses_stacks_that_would_reach_too_far_and_changes_nothing() {
    let (mut doc, l) = world();
    let base = l[0];
    for _ in 0..3 {
        doc.add_filter(
            base,
            FilterTarget::Content,
            FilterSpec::new(EffectSettings::blur(150)).channels(&[Channel::Color]),
        )
        .unwrap();
    }
    doc.clear_history().unwrap();
    let (revision, width, steps, before) =
        (doc.revision(), doc.width(), doc.undo_count(), colour(&doc));
    // 1.5 倍で 150 → 225 が 3 つ = 675 > 512
    assert!(matches!(
        doc.resize_image(60, 42, CanvasResampling::Nearest),
        Err(CoreError::Unsupported(_))
    ));
    assert_eq!(doc.width(), width);
    assert_eq!(doc.revision(), revision);
    assert_eq!(doc.undo_count(), steps);
    assert_eq!(colour(&doc), before);
    assert_eq!(blur_radii(&doc, base), vec![150, 150, 150]);
}

fn plane() -> SurfaceGeometry {
    let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y);
    SurfaceGeometry::new(
        vec![
            SurfaceTriangle::new(a, b, c, Vec2::ZERO, Vec2::X, Vec2::ONE),
            SurfaceTriangle::new(a, c, d, Vec2::ZERO, Vec2::ONE, Vec2::Y),
        ],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap()
}

fn options(width: u32, height: u32) -> Options<'static> {
    Options {
        width,
        height,
        tile_size: 8,
        ..Options::default()
    }
}

fn canvas_path_of(doc: &Document, layer: LayerId) -> yolu_core::paths::CanvasPath {
    match doc.layer(layer).unwrap().path() {
        Some(LayerPath::Canvas(p)) => p.clone(),
        other => panic!("{other:?}"),
    }
}

/// 画像のサイズ変更は、2D のパスの点とブラシの半径を倍率に合わせ、パスから描き直す（写した画素でなく）。モデルの上のパスは残り、
/// 報告にレイヤーを挙げる（呼び手が描き直す）。Undo でパスも画素も戻る。
#[test]
fn resizing_redraws_2d_paths_from_the_scaled_path_and_lists_the_3d_ones() {
    let mut r = rig();
    let g = plane();
    let surface_layer = r.doc.add_layer("面のパス").unwrap();
    r.doc
        .set_channel_enabled(surface_layer, Channel::Height, true)
        .unwrap();
    let path = SurfacePath {
        style: Default::default(),
        id: 9,
        channel: Channel::Height,
        brush: yolu_core::paths::PathBrush(yolu_core::BrushSettings {
            radius: 0.1,
            ..Default::default()
        }),
        points: vec![
            PathPoint::new(0, 0.2, 0.3, 1.0).unwrap(),
            PathPoint::new(1, 0.4, 0.2, 1.0).unwrap(),
        ],
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    let drawn = render_surface(&path, &g, &options(W, H)).unwrap();
    r.doc
        .set_path(surface_layer, LayerPath::Surface(path), drawn.channels)
        .unwrap();
    r.doc.clear_history().unwrap();
    let old = canvas_path_of(&r.doc, r.path_layer);
    let old_pixels = r
        .doc
        .layer(r.path_layer)
        .unwrap()
        .surface(Channel::Color)
        .unwrap()
        .to_canvas_bytes();
    let report = r
        .doc
        .resize_image(80, 56, CanvasResampling::Bilinear)
        .unwrap();
    assert_eq!(report.surface_path_layers, vec![surface_layer]);
    let scaled = canvas_path_of(&r.doc, r.path_layer);
    assert_eq!(scaled.id, old.id);
    assert_eq!(scaled.brush.0.radius, old.brush.0.radius * 2.0);
    for (a, b) in scaled.points.iter().zip(&old.points) {
        assert_eq!((a.x, a.y, a.pressure), (b.x * 2.0, b.y * 2.0, b.pressure));
    }
    // 画素はパスから描き直したもの（拡大して写したものではない）
    let redrawn = render_canvas(&scaled, &options(80, 56)).unwrap();
    let layer = r.doc.layer(r.path_layer).unwrap();
    assert_eq!(layer.surface(Channel::Color).unwrap().width(), 80);
    assert_eq!(
        layer.surface(Channel::Color).unwrap().to_canvas_bytes(),
        redrawn.channels[0].1.to_canvas_bytes()
    );
    assert_eq!(
        r.doc
            .layer(surface_layer)
            .unwrap()
            .surface(Channel::Height)
            .unwrap()
            .width(),
        80
    );
    r.doc.undo().unwrap();
    assert_eq!(canvas_path_of(&r.doc, r.path_layer), old);
    assert_eq!(
        r.doc
            .layer(r.path_layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        old_pixels
    );
}

/// キャンバスだけを動かす（resize_canvas）ときも、2D のパスは点をずらして描き直し、レイヤーの画素と食い違わない。
#[test]
fn resizing_the_canvas_moves_2d_path_points_and_redraws() {
    let mut r = rig();
    r.doc.clear_history().unwrap();
    let old = canvas_path_of(&r.doc, r.path_layer);
    r.doc.resize_canvas(48, 36, (4, 3)).unwrap();
    let moved = canvas_path_of(&r.doc, r.path_layer);
    for (a, b) in moved.points.iter().zip(&old.points) {
        assert_eq!((a.x, a.y), (b.x + 4.0, b.y + 3.0));
    }
    assert_eq!(moved.brush, old.brush);
    let redrawn = render_canvas(&moved, &options(48, 36)).unwrap();
    assert_eq!(
        r.doc
            .layer(r.path_layer)
            .unwrap()
            .surface(Channel::Color)
            .unwrap()
            .to_canvas_bytes(),
        redrawn.channels[0].1.to_canvas_bytes()
    );
    r.doc.undo().unwrap();
    assert_eq!(canvas_path_of(&r.doc, r.path_layer), old);
}

/// 対称のある 2D のパスは、拡大・縮小でもキャンバスだけを動かすときでも、対称の中心を点と同じに動かして描き直す: 映した側の画素が、
/// 新しいキャンバスの中心に対して左右・上下に対称のまま残る。
#[test]
fn resizing_moves_the_symmetry_centre_of_a_2d_path_with_its_points() {
    use yolu_core::glam::DVec2;
    use yolu_core::paths::{CanvasPoint, PathStyle, PathSymmetry};
    use yolu_core::{CanvasSymmetry, SymmetryMode};
    let build = || {
        let mut doc = Document::with_tile_size(W, H, 8).unwrap();
        let layer = doc.add_layer("対称のパス").unwrap();
        let mut path = path_points(0.0);
        // 映した側と重ならないよう、左上の四分の一に収める（重なると合成の順で最後の 1 が揺れる）
        path.points = vec![
            CanvasPoint::new(4.5, 3.25, 0.5).unwrap(),
            CanvasPoint::new(11.5, 8.5, 1.0).unwrap(),
            CanvasPoint::new(15.25, 4.75, 0.7).unwrap(),
        ];
        path.style = PathStyle {
            symmetry: PathSymmetry::Canvas(
                CanvasSymmetry::new(
                    SymmetryMode::Both,
                    DVec2::new(W as f64 / 2.0, H as f64 / 2.0),
                    2,
                )
                .unwrap(),
            ),
            ..Default::default()
        };
        doc.set_canvas_path(layer, path).unwrap();
        doc.clear_history().unwrap();
        (doc, layer)
    };
    let centre = |doc: &Document, layer: LayerId| match canvas_path_of(doc, layer).style.symmetry {
        PathSymmetry::Canvas(s) => s.center,
        other => panic!("{other:?}"),
    };
    // 左右・上下に対称な画素（キャンバスの中心は画素の境目）
    let assert_symmetric = |doc: &Document, layer: LayerId, what: &str| {
        let surface = doc.layer(layer).unwrap().surface(Channel::Color).unwrap();
        let (w, h) = (surface.width(), surface.height());
        let bytes = surface.to_canvas_bytes();
        let alpha = |x: u32, y: u32| bytes[((y * w + x) * 4 + 3) as usize];
        let mut drawn = 0;
        for y in 0..h {
            for x in 0..w {
                let a = alpha(x, y);
                drawn += usize::from(a > 0);
                assert_eq!(alpha(w - 1 - x, y), a, "{what}: 左右 ({x}, {y})");
                assert_eq!(alpha(x, h - 1 - y), a, "{what}: 上下 ({x}, {y})");
            }
        }
        assert!(drawn > 0, "{what}: 何も描かれていない");
    };
    let (mut doc, layer) = build();
    assert_symmetric(&doc, layer, "元の大きさ");
    // 縦横で倍率が違う拡大（2 倍と 1.5 倍）: 中心も縦横の倍率で動く
    doc.resize_image(W * 2, H * 3 / 2, CanvasResampling::Bilinear)
        .unwrap();
    assert_eq!(
        centre(&doc, layer),
        DVec2::new(W as f64, H as f64 * 3.0 / 4.0)
    );
    assert_symmetric(&doc, layer, "拡大のあと");
    doc.undo().unwrap();
    assert_eq!(
        centre(&doc, layer),
        DVec2::new(W as f64 / 2.0, H as f64 / 2.0),
        "Undo で中心も戻る"
    );
    // 縮小
    doc.resize_image(W / 2, H / 2, CanvasResampling::Area)
        .unwrap();
    assert_eq!(
        centre(&doc, layer),
        DVec2::new(W as f64 / 4.0, H as f64 / 4.0)
    );
    assert_symmetric(&doc, layer, "縮小のあと");
    // キャンバスだけを広げる（点を画素と同じだけずらす）: 中心も同じだけずれる。新しいキャンバスの中心になるよう左右・上下へ均等に
    let (mut doc, layer) = build();
    doc.resize_canvas(W + 8, H + 6, (4, 3)).unwrap();
    assert_eq!(
        centre(&doc, layer),
        DVec2::new(W as f64 / 2.0 + 4.0, H as f64 / 2.0 + 3.0)
    );
    assert_symmetric(&doc, layer, "キャンバスを広げたあと");
}

/// 線対称（斜めの鏡）のある 2D のパスは、縦横の倍率が違う拡大で、中心と一緒に軸の向きも倍率で動かす（軸の向きのベクトルを倍率で写す）。
/// 倍率が同じなら向きは変わらない。
#[test]
fn resizing_turns_the_axis_of_a_lines_symmetry_path_with_the_scales() {
    use yolu_core::glam::DVec2;
    use yolu_core::paths::{CanvasPoint, PathStyle, PathSymmetry};
    use yolu_core::CanvasSymmetry;
    let build = || {
        let mut doc = Document::with_tile_size(W, H, 8).unwrap();
        let layer = doc.add_layer("線対称のパス").unwrap();
        let mut path = path_points(0.0);
        path.points = vec![
            CanvasPoint::new(2.5, 3.25, 0.5).unwrap(),
            CanvasPoint::new(5.5, 4.5, 1.0).unwrap(),
        ];
        path.style = PathStyle {
            symmetry: PathSymmetry::Canvas(
                CanvasSymmetry::lines(DVec2::new(W as f64 / 2.0, H as f64 / 2.0), 2, 45.0).unwrap(),
            ),
            ..Default::default()
        };
        doc.set_canvas_path(layer, path).unwrap();
        doc.clear_history().unwrap();
        (doc, layer)
    };
    let symmetry = |doc: &Document, layer: LayerId| match canvas_path_of(doc, layer).style.symmetry
    {
        PathSymmetry::Canvas(s) => s,
        other => panic!("{other:?}"),
    };
    // 横 2 倍・縦 1 倍: 45 度の軸の向き (1, 1) は (2, 1) になる
    let (mut doc, layer) = build();
    doc.resize_image(W * 2, H, CanvasResampling::Bilinear)
        .unwrap();
    let s = symmetry(&doc, layer);
    assert_eq!(s.center, DVec2::new(W as f64, H as f64 / 2.0));
    assert!(
        (s.angle - (1.0f64).atan2(2.0).to_degrees()).abs() < 1e-9,
        "{}",
        s.angle
    );
    assert!(s.validate().is_ok());
    doc.undo().unwrap();
    assert_eq!(symmetry(&doc, layer).angle, 45.0, "Undo で向きも戻る");
    // 倍率が同じなら向きは変わらない
    doc.resize_image(W * 2, H * 2, CanvasResampling::Bilinear)
        .unwrap();
    assert_eq!(symmetry(&doc, layer).angle, 45.0);
    // キャンバスだけを動かすときも向きは変わらない
    let (mut doc, layer) = build();
    doc.resize_canvas(W + 8, H + 6, (4, 3)).unwrap();
    assert_eq!(symmetry(&doc, layer).angle, 45.0);
}

/// 大きさを変えると、元の画素の無いレイヤー（塗りつぶし＋フィルター）の出力も新しい大きさになる: キャンバスの端の欠けたタイルで評価した古い
/// 出力を返さない。
#[test]
fn a_canvas_size_change_never_serves_the_old_sizes_output() {
    let (mut doc, l) = world();
    let fill = l[3];
    doc.add_filter(
        fill,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::invert()).channels(&[Channel::Color]),
    )
    .unwrap();
    doc.clear_history().unwrap();
    // 28 行 = 3.5 タイル: いちばん上のタイルは欠けている。評価してキャッシュへ入れてから、キャンバスを 32 行へ
    let _ = colour(&doc);
    doc.resize_canvas(40, 32, (0, 0)).unwrap();
    assert_cache_is_honest(&doc, "キャンバスを伸ばしたあと");
    let top_row = colour(&doc);
    let last = (31 * 40) * 4;
    assert!(top_row[last + 3] > 0, "伸ばした行にも塗りつぶしが出る");
    doc.undo().unwrap();
    assert_cache_is_honest(&doc, "取り消したあと");
}

/// パスで描かれたチャンネルを無効にできるのは、組（material）を持たないパスでないときだけ断る（C# の `Path.Material == null`）:
/// 組を持つパスは、基準のチャンネルも組のチャンネルも無効にでき、取り消しで戻る。組を持たないパスは断って何も変えない。
#[test]
fn a_material_path_layers_channels_can_be_disabled_but_a_plain_paths_cannot() {
    let mut r = rig();
    // 組を持たないパス（rig のレイヤー）: 描くチャンネルは無効にできない
    let before = state_of(&r.doc);
    assert!(matches!(
        r.doc
            .set_channel_enabled(r.path_layer, Channel::Color, false),
        Err(CoreError::Unsupported(_))
    ));
    assert!(state_of(&r.doc) == before);
    // 組を持たないパスでも、描かないチャンネルは無効にできる
    r.doc
        .set_channel_enabled(r.path_layer, Channel::Height, true)
        .unwrap();
    r.doc
        .set_channel_enabled(r.path_layer, Channel::Height, false)
        .unwrap();
    // 組を持つパスへ付け替える（基準のチャンネルは Color のまま）
    let mut path = path_points(0.0);
    path.material = Some(vec![
        yolu_core::paths::ChannelPaint {
            channel: Channel::Color,
            color: Rgba8::new(10, 200, 30, 255),
        },
        yolu_core::paths::ChannelPaint {
            channel: Channel::Emission,
            color: Rgba8::new(90, 90, 90, 255),
        },
    ]);
    r.doc.set_canvas_path(r.path_layer, path).unwrap();
    for channel in [Channel::Color, Channel::Emission] {
        let steps = r.doc.undo_count();
        r.doc
            .set_channel_enabled(r.path_layer, channel, false)
            .unwrap();
        assert!(!r
            .doc
            .layer(r.path_layer)
            .unwrap()
            .is_channel_enabled(channel));
        assert!(r.doc.layer(r.path_layer).unwrap().path().is_some());
        assert_eq!(r.doc.undo_count(), steps + 1);
        r.doc.undo().unwrap();
        assert!(r
            .doc
            .layer(r.path_layer)
            .unwrap()
            .is_channel_enabled(channel));
    }
}

/// 画素を変えない交換（複数のレイヤーの表示の切り替え・元に戻す・やり直す）は、評価したぼかしのキャッシュを捨てない: 再評価しない。
/// 画素を変える交換（変形）は、変わったレイヤーだけ作り直す（古い出力を返さない）。
#[test]
fn swaps_that_leave_the_pixels_alone_keep_the_evaluation_cache() {
    let mut doc = plain_doc();
    let (lower, upper) = painted_pair(&mut doc);
    doc.add_filter(
        lower,
        FilterTarget::Content,
        FilterSpec::new(EffectSettings::blur(4)).channels(&[Channel::Color]),
    )
    .unwrap();
    doc.clear_history().unwrap();
    let shown = colour(&doc);
    let evaluated = doc.effect_counters().blocks_evaluated;
    assert!(evaluated > 0);
    // 複数のレイヤーの表示を切り替える（ぼかしを持つレイヤーも含む）と戻す。2 回の交換とその Undo・Redo
    doc.set_layers_visibility(&[lower, upper], false).unwrap();
    doc.set_layers_visibility(&[lower, upper], true).unwrap();
    assert_eq!(colour(&doc), shown);
    for _ in 0..2 {
        doc.undo().unwrap();
    }
    for _ in 0..2 {
        doc.redo().unwrap();
    }
    assert_eq!(colour(&doc), shown);
    assert_eq!(
        doc.effect_counters().blocks_evaluated,
        evaluated,
        "画素を変えない交換で、評価のキャッシュを捨てて再評価した"
    );
    assert_cache_is_honest(&doc, "表示を切り替えたあと");
    // 画素を変える交換は、変わったレイヤーの出力を作り直す
    doc.transform_layers(
        &[lower],
        Affine2D::translation(3., -2.),
        Resampling::Nearest,
    )
    .unwrap();
    let moved = colour(&doc);
    assert_ne!(moved, shown);
    assert!(doc.effect_counters().blocks_evaluated > evaluated);
    assert_cache_is_honest(&doc, "変形のあと");
    doc.undo().unwrap();
    assert_eq!(colour(&doc), shown);
    assert_cache_is_honest(&doc, "変形を取り消したあと");
}

#[test]
fn cutting_pixels_from_a_path_layer_is_refused_and_changes_nothing() {
    // パスレイヤーの画素はパスが決めるので切り取らない（C# の CutPixels）。断ってもクリップボードの元・文書・履歴は変わらない
    let Rig {
        mut doc,
        path_layer,
        ..
    } = rig();
    let before = doc.composite(doc.bounds()).unwrap();
    let undo = doc.undo_count();
    assert!(doc
        .cut_pixels(path_layer, Channel::Color, false, u64::MAX)
        .is_err());
    assert_eq!(doc.composite(doc.bounds()).unwrap(), before);
    assert_eq!(doc.undo_count(), undo);
    // 写すだけなら断らない（画素を変えない）
    assert!(doc
        .copy_pixels(path_layer, Channel::Color, false, u64::MAX)
        .is_ok());
}

#[test]
fn a_recovery_snapshot_keeps_the_effects_and_composites_the_same() {
    // 復旧の書き置きの写し（capture_snapshot）は効果の入力と予算を写し、評価のキャッシュは写さない。写しの合成は元と同じ
    let Rig { doc, .. } = rig();
    let copy = doc.capture_snapshot().unwrap();
    assert_eq!(
        copy.composite(copy.bounds()).unwrap(),
        doc.composite(doc.bounds()).unwrap()
    );
    for channel in [Channel::Color, Channel::Height] {
        assert_eq!(whole(&copy, channel), whole(&doc, channel), "{channel:?}");
    }
}
