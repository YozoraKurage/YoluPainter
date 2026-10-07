//! 重なった UV のテクセルの持ち主の決め方（ベイクの優先）と、重なったテクセルの図（`uv_overlap`）。
//!
//! 試験のモデル: 同じ UV（0.25〜0.75 の正方形）に重ねた 2 枚の四角（三角形 0・1 が −X の側の小さな四角、2・3 が +X の側の大きな
//! 四角）。位置のマップの x で、どちらの四角が焼かれたかが分かる。
use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use yolu_core::mesh_maps::*;

/// z = 0 の四角（x0〜x1・y0〜y1）を 2 つの三角形にし、UV の正方形 `uv`（u0, v0, u1, v1）に写す。
fn quad(x0: f32, x1: f32, y0: f32, y1: f32, uv: [f32; 4]) -> (Vec<f32>, Vec<f32>) {
    let [u0, v0, u1, v1] = uv;
    let corners = vec![
        x0, y0, 0., x1, y0, 0., x0, y1, 0., // 三角形 1 つ目
        x0, y1, 0., x1, y0, 0., x1, y1, 0., // 2 つ目
    ];
    let uvs = vec![u0, v0, u1, v0, u0, v1, u0, v1, u1, v0, u1, v1];
    (corners, uvs)
}

const SQUARE: [f32; 4] = [0.25, 0.25, 0.75, 0.75];

/// −X の小さな四角（三角形 0・1）と +X の大きな四角（2・3）。`extra` があれば三角形 4・5 にその UV の四角を足す（x は 5〜6）。
fn model(extra: Option<[f32; 4]>) -> MeshBakeInput {
    let (mut corners, mut uvs) = quad(-2., -1., 0., 1., SQUARE);
    let (c, u) = quad(1., 3., 0., 2., SQUARE);
    corners.extend(c);
    uvs.extend(u);
    if let Some(uv) = extra {
        let (c, u) = quad(5., 6., 0., 1., uv);
        corners.extend(c);
        uvs.extend(u);
    }
    let n = corners.len() / 9;
    MeshBakeInput::new(corners, uvs, vec![0; n], MeshBakeAttributes::default()).unwrap()
}

fn settings(overlap: MeshOverlapPriority) -> MeshBakeSettings {
    MeshBakeSettings {
        width: 16,
        height: 16,
        padding: 0,
        maps: vec![MeshMapKind::Position, MeshMapKind::WorldNormal],
        overlap,
        ..Default::default()
    }
}

fn run(input: &MeshBakeInput, s: &MeshBakeSettings) -> MeshBakeResult {
    let r = bake(input, s, &MeshBakeBudget::default(), None, None, |_, _| {
        true
    })
    .unwrap();
    assert_eq!(r.status, MeshBakeStatus::Completed);
    r
}

fn bake_error(input: &MeshBakeInput, s: &MeshBakeSettings) -> String {
    bake(input, s, &MeshBakeBudget::default(), None, None, |_, _| {
        true
    })
    .err()
    .expect("断られるはずです")
    .to_string()
}

/// UV の正方形の真ん中のテクセルの位置のマップの x（0 は境界箱の −X の端）。
fn centre_x(r: &MeshBakeResult) -> u16 {
    let map = r
        .maps
        .iter()
        .find(|m| m.kind() == MeshMapKind::Position)
        .unwrap();
    map.raw_value(8, 8, 0).unwrap()
}

fn rule(rule: MeshOverlapRule) -> MeshOverlapPriority {
    let mut p = MeshOverlapPriority::default();
    p.rule = rule;
    p
}

fn outside() -> MeshOverlapPriority {
    let mut p = MeshOverlapPriority::default();
    p.skip_outside = true;
    p
}

/// 位置のマップの x が −X の小さな四角（境界箱 −2〜3 の左 1/5）か。
fn is_left(x: u16) -> bool {
    x < 65535 / 5
}

#[test]
fn the_default_keeps_the_lowest_triangle_and_the_same_keys_and_bytes() {
    let input = model(None);
    let default = run(&input, &settings(MeshOverlapPriority::default()));
    assert!(is_left(centre_x(&default)), "番号の小さい −X の四角");
    assert!(default.report.overlap_texels > 0);
    // 由来の鍵に何も足さない（前に焼いたマップが古くならない）
    let normal = default
        .maps
        .iter()
        .find(|m| m.kind() == MeshMapKind::WorldNormal)
        .unwrap();
    assert_eq!(normal.provenance().settings_key, "source=vertex-normals");
    // 重ならない島（三角形 4・5）を優先して並べ替えの道を通しても、重なったテクセルの持ち主は変わらず、焼いた値と覆いは同じバイト
    let apart = model(Some([0.0, 0.0, 0.2, 0.2]));
    let before = run(&apart, &settings(MeshOverlapPriority::default()));
    let prefer = MeshOverlapPriority::default()
        .with_island(apart.topology_hash(), 4, Some(MeshOverlapList::Prefer))
        .unwrap();
    let after = run(&apart, &settings(prefer));
    for (a, b) in before.maps.iter().zip(&after.maps) {
        assert_eq!(a.data(), b.data());
        assert_eq!(a.coverage(), b.coverage());
        assert_ne!(a.provenance().settings_key, b.provenance().settings_key);
    }
}

#[test]
fn each_rule_picks_its_owner_and_changes_the_condition_key() {
    let input = model(None);
    let default = run(&input, &settings(MeshOverlapPriority::default()));
    for (r, left) in [
        (MeshOverlapRule::LowestIndex, true),
        (MeshOverlapRule::LargerArea, false),
        (MeshOverlapRule::PositiveX, false),
        (MeshOverlapRule::NegativeX, true),
    ] {
        let s = settings(rule(r));
        let baked = run(&input, &s);
        assert_eq!(is_left(centre_x(&baked)), left, "{r:?}");
        // 重なりの数（覆いの印）は持ち主によらない
        assert_eq!(
            baked.report.overlap_texels, default.report.overlap_texels,
            "{r:?}"
        );
        let key = s.kind_key(MeshMapKind::Position, "");
        if r == MeshOverlapRule::LowestIndex {
            assert_eq!(key, "normalize=bounding-box");
        } else {
            assert_eq!(key, format!("normalize=bounding-box;owner={}", r.name()));
            // 前の決め方で焼いたマップは「設定が変わった」で古い
            let expect = MeshMapExpectation::for_bake(&input, &s, None);
            let check = default.maps[0].provenance().check(&expect);
            assert_eq!(check.state, MeshMapState::Stale, "{r:?}");
        }
    }
}

#[test]
fn hand_picked_islands_are_preferred_or_not_baked() {
    let input = model(None);
    let binding = input.topology_hash().to_owned();
    // +X の四角（三角形 2 の島）を優先する
    let prefer = MeshOverlapPriority::default()
        .with_island(&binding, 2, Some(MeshOverlapList::Prefer))
        .unwrap();
    assert_eq!(prefer.preferred(), &BTreeSet::from([2]));
    assert!(!is_left(centre_x(&run(&input, &settings(prefer.clone())))));
    // 優先は自動の決め方より先（−X を優先する決め方でも、手で選んだ +X が勝つ）
    let mut both = prefer.clone();
    both.rule = MeshOverlapRule::NegativeX;
    assert!(!is_left(centre_x(&run(&input, &settings(both)))));
    // −X の四角（三角形 1 の島。島のどの三角形でもよい）を焼かない: +X だけが焼かれ、重なりも無くなる
    let skip = MeshOverlapPriority::default()
        .with_island(&binding, 1, Some(MeshOverlapList::Skip))
        .unwrap();
    let r = run(&input, &settings(skip.clone()));
    assert!(!is_left(centre_x(&r)));
    assert_eq!(r.report.overlap_texels, 0);
    assert_eq!(r.report.receiving_triangles, 2);
    // 同じ島を反対の一覧に入れると、もう一方から外れる。None で両方から外れ、空になれば既定に戻る
    let moved = skip
        .with_island(&binding, 1, Some(MeshOverlapList::Prefer))
        .unwrap();
    assert!(moved.skipped().is_empty() && moved.preferred().contains(&1));
    let cleared = moved.with_island("", 1, None).unwrap();
    assert!(cleared.is_default() && cleared.binding().is_empty());
    // 鍵は一覧で変わる
    let a = settings(skip.clone()).kind_key(MeshMapKind::Position, "");
    let b = settings(prefer.clone()).kind_key(MeshMapKind::Position, "");
    assert!(a.contains(";islands=") && b.contains(";islands=") && a != b);
}

#[test]
fn hand_picked_islands_of_another_model_are_refused() {
    let input = model(None);
    let other = model(Some(SQUARE));
    let skip = MeshOverlapPriority::default()
        .with_island(other.topology_hash(), 0, Some(MeshOverlapList::Skip))
        .unwrap();
    assert_eq!(
        bake_error(&input, &settings(skip.clone())),
        "手で選んだ島が別のモデルに属しています"
    );
    // 別のモデルの一覧へ足すのも断る（外すのは受ける）
    assert!(skip
        .with_island(input.topology_hash(), 2, Some(MeshOverlapList::Prefer))
        .is_err());
    assert!(skip.with_island(input.topology_hash(), 0, None).is_ok());
    // 指紋が正しくても、モデルに無い三角形の番号は断る
    let far = MeshOverlapPriority::default()
        .with_island(input.topology_hash(), 99, Some(MeshOverlapList::Skip))
        .unwrap();
    assert_eq!(
        bake_error(&input, &settings(far)),
        "手で選んだ島の番号がモデルにありません"
    );
    // 型・範囲の拒否
    assert!(MeshOverlapPriority::new(
        MeshOverlapRule::LowestIndex,
        false,
        "nothex".into(),
        BTreeSet::from([0]),
        BTreeSet::new()
    )
    .is_err());
    assert!(MeshOverlapPriority::new(
        MeshOverlapRule::LowestIndex,
        false,
        input.topology_hash().into(),
        BTreeSet::from([0]),
        BTreeSet::from([0])
    )
    .is_err());
    assert!(MeshOverlapPriority::new(
        MeshOverlapRule::LowestIndex,
        false,
        input.topology_hash().into(),
        (0..=MAX_OVERLAP_ISLANDS).collect(),
        BTreeSet::new()
    )
    .is_err());
    assert_eq!(MeshOverlapRule::from_index(4), None);
    assert_eq!(
        MeshOverlapRule::from_index(2),
        Some(MeshOverlapRule::PositiveX)
    );
}

#[test]
fn islands_moved_outside_the_unit_square_are_skipped_only_when_asked() {
    // 0〜1 の外（u が 1.25〜1.75）へずらした島。今までどおり、既定では断る
    let input = model(Some([1.25, 0.25, 1.75, 0.75]));
    assert_eq!(
        bake_error(&input, &settings(MeshOverlapPriority::default())),
        "UVが0〜1の外です。繰り返し・UDIMのUVはベイクできません"
    );
    let skip_outside = outside();
    let r = run(&input, &settings(skip_outside.clone()));
    assert_eq!(r.report.receiving_triangles, 4, "外の島の 2 つを除く");
    // 外の島を除いた結果は、外の島が無いモデルと同じ値（位置の正規化の箱は外の島の位置も含むので、法線で比べる）
    let without = run(&model(None), &settings(skip_outside.clone()));
    let normal = |r: &MeshBakeResult| {
        r.maps
            .iter()
            .find(|m| m.kind() == MeshMapKind::WorldNormal)
            .unwrap()
            .data()
            .to_vec()
    };
    assert_eq!(normal(&r), normal(&without));
    assert_eq!(
        settings(skip_outside).kind_key(MeshMapKind::WorldNormal, ""),
        "source=vertex-normals;outside=skip"
    );
    // 0〜1 をまたぐ島は外へずらした島ではないので、選んでいても断る
    let straddle = model(Some([0.75, 0.25, 1.25, 0.75]));
    let s = settings(outside());
    assert_eq!(
        bake_error(&straddle, &s),
        "UVが0〜1の外です。繰り返し・UDIMのUVはベイクできません"
    );
    // 負の側（v が −1〜0）へずらした島も外
    let below = model(Some([0.25, -0.75, 0.75, -0.25]));
    assert_eq!(run(&below, &s).report.receiving_triangles, 4);
}

#[test]
fn the_cpu_plan_hands_receivers_in_the_owner_order() {
    // GPU などの別の実行場所へ渡す受け手の並びも、持ち主の順（+X の四角が先）
    let input = model(None);
    let s = settings(rule(MeshOverlapRule::PositiveX));
    let budget = MeshBakeBudget::default();
    let MeshBakePlanOutcome::Ready(plan) =
        MeshBakePlan::prepare(&input, &s, &budget, None, None, &mut |_, _| true).unwrap()
    else {
        panic!("準備で止まった");
    };
    assert_eq!(plan.scene().receivers, vec![2, 3, 0, 1]);
}

#[test]
fn the_overlap_map_counts_the_same_texels_as_the_bake() {
    let input = model(Some([0.0, 0.0, 0.2, 0.2]));
    let s = MeshBakeSettings {
        antialiasing: 1,
        ..settings(MeshOverlapPriority::default())
    };
    let baked = run(&input, &s);
    let uvs: Vec<f32> = [
        quad(-2., -1., 0., 1., SQUARE).1,
        quad(1., 3., 0., 2., SQUARE).1,
        quad(5., 6., 0., 1., [0.0, 0.0, 0.2, 0.2]).1,
    ]
    .concat();
    let all: Vec<usize> = (0..6).collect();
    let map = uv_overlap(&uvs, &all, 16, 16, None).unwrap().unwrap();
    assert_eq!(map.texel_count(), baked.report.overlap_texels);
    assert_eq!(map.triangles(), &[0, 1, 2, 3], "重ならない島は入らない");
    let coverage = baked.maps[0].coverage();
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(
                map.contains(x, y),
                coverage[(y * 16 + x) as usize] == 2,
                "({x}, {y})"
            );
        }
    }
    assert!(map.any_in(8, 8, 8, 8) && !map.any_in(0, 0, 2, 2));
    assert!(map.any_in(-100, -100, 100, 100) && !map.any_in(20, 20, 30, 30));
    // 片方の四角だけなら重ならない
    let one = uv_overlap(&uvs, &[0, 1, 4, 5], 16, 16, None)
        .unwrap()
        .unwrap();
    assert!(one.is_empty() && one.triangles().is_empty());
    // 取り消し
    let cancel = AtomicBool::new(true);
    assert_eq!(uv_overlap(&uvs, &all, 16, 16, Some(&cancel)).unwrap(), None);
    // 範囲の外・番号の外
    assert!(uv_overlap(&uvs, &all, 0, 16, None).is_err());
    assert!(uv_overlap(&uvs, &[6], 16, 16, None).is_err());
}

#[test]
fn bake_islands_split_mirrored_halves_that_share_the_same_uvs() {
    // UV だけでつなぐと（ID の UV アイランド）重なった 2 枚の四角は 1 つになるが、優先の島は 3D の位置でも分ける
    let input = model(Some([0.0, 0.0, 0.2, 0.2]));
    assert_eq!(bake_islands(&input), vec![0, 0, 1, 1, 2, 2]);
    // UV の継ぎ目（同じ位置で UV が違う辺）でも分かれる: 四角の 2 つ目の三角形だけ UV をずらす
    let (corners, mut uvs) = quad(0., 1., 0., 1., SQUARE);
    for v in uvs[6..].iter_mut() {
        *v += 0.1;
    }
    let seam = MeshBakeInput::new(corners, uvs, vec![0, 0], MeshBakeAttributes::default()).unwrap();
    assert_eq!(bake_islands(&seam), vec![0, 1]);
}

#[test]
fn the_document_keeps_the_priority_with_one_undo_step() {
    use yolu_core::Document;
    let input = model(None);
    let mut doc = Document::with_tile_size(16, 16, 8).unwrap();
    assert!(doc.bake_priority().is_default());
    let p = MeshOverlapPriority::default()
        .with_island(input.topology_hash(), 2, Some(MeshOverlapList::Skip))
        .unwrap();
    doc.set_bake_priority(p.clone()).unwrap();
    assert_eq!(doc.undo_count(), 1);
    // 同じ値は段を積まない
    doc.set_bake_priority(p.clone()).unwrap();
    assert_eq!(doc.undo_count(), 1);
    // 写しにも入る（書き出し・保存の写しが同じ値を読む）
    assert_eq!(doc.capture_snapshot().unwrap().bake_priority(), &p);
    doc.undo().unwrap();
    assert!(doc.bake_priority().is_default());
    doc.redo().unwrap();
    assert_eq!(doc.bake_priority(), &p);
    // 読み込みの直後の戻しは履歴を増やさない。履歴のある文書には使えない
    let mut fresh = Document::with_tile_size(16, 16, 8).unwrap();
    fresh.restore_bake_priority(p.clone()).unwrap();
    assert_eq!((fresh.undo_count(), fresh.bake_priority()), (0, &p));
    assert!(doc
        .restore_bake_priority(MeshOverlapPriority::default())
        .is_err());
}
