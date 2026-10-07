//! グループの入れ子の上限（`MAX_GROUP_DEPTH`）と、親子の確かめがレイヤーの数に対して線形であること。
//! 合成・書き出しはグループの入れ子を再帰でたどる（スタックの実測は `MAX_GROUP_DEPTH` の説明）ので、上限を超える入れ子は、読み込みでも編集でも作らせない。

use std::time::{Duration, Instant};
use yolu_core::smart::SmartPlacement;
use yolu_core::{CoreError, Document, LayerId, Rect, MAX_GROUP_DEPTH};

fn doc() -> Document {
    Document::with_tile_size(8, 8, 8).unwrap()
}

/// ラスターレイヤー 1 枚を、`groups` 個のグループの鎖（外側ほど後）で包んだ文書。一番外側のグループの ID も返す。
fn chain(groups: usize) -> (Document, LayerId) {
    let mut d = doc();
    let mut top = d.add_layer("leaf").unwrap();
    for i in 0..groups {
        top = d.group_layers(&[top], &format!("g{i}")).unwrap();
    }
    d.clear_history().unwrap();
    (d, top)
}

fn too_deep(e: &CoreError) -> bool {
    matches!(e, CoreError::InvalidArgument(why) if why.contains("入れ子が深すぎる"))
}

#[test]
fn the_limit_is_64_groups_on_one_chain() {
    assert_eq!(MAX_GROUP_DEPTH, 64);
}

#[test]
fn grouping_stops_at_the_depth_limit_and_changes_nothing() {
    let (mut d, top) = chain(MAX_GROUP_DEPTH);
    assert_eq!(d.depth_of(d.layers()[0].id()).unwrap(), MAX_GROUP_DEPTH);
    let (layers, revision) = (d.layers().len(), d.revision());
    let e = d.group_layers(&[top], "one more").unwrap_err();
    assert!(too_deep(&e), "{e:?}");
    assert_eq!(
        (d.layers().len(), d.revision(), d.undo_count()),
        (layers, revision, 0)
    );
    // 別のレイヤーをまとめるだけなら、深さは増えないので通る
    let extra = d.add_layer("extra").unwrap();
    d.group_layers(&[extra], "side").unwrap();
    d.validate_structure().unwrap();
}

#[test]
fn moving_a_group_into_a_deep_chain_is_refused_but_a_plain_layer_fits() {
    let (mut d, top) = chain(MAX_GROUP_DEPTH);
    let innermost = d.layers()[1].id();
    assert!(d.layer(innermost).unwrap().is_group());
    // もう 1 つのグループを一番内側へ入れると 65 段になる
    let other = d.add_group("other", None).unwrap();
    let before: Vec<_> = d.layers().iter().map(|l| (l.id(), l.parent())).collect();
    let e = d.move_layer_to(other, Some(innermost), 0).unwrap_err();
    assert!(too_deep(&e), "{e:?}");
    let e = d.move_layers(&[other], Some(innermost), 0).unwrap_err();
    assert!(too_deep(&e), "{e:?}");
    let after: Vec<_> = d.layers().iter().map(|l| (l.id(), l.parent())).collect();
    assert_eq!(before, after, "断ったら何も変えない");
    // ラスターレイヤーは 64 個のグループの中へ入れられる（レイヤーの深さは 65 になるが、鎖のグループは 64）
    let plain = d.add_layer("plain").unwrap();
    d.move_layer_to(plain, Some(innermost), 0).unwrap();
    assert_eq!(d.depth_of(plain).unwrap(), MAX_GROUP_DEPTH);
    d.validate_structure().unwrap();
    // 鎖ごと別のグループの中へ入れるのも、深くなるなら断る
    let shallow = d.add_group("shallow", None).unwrap();
    let e = d.move_layer_to(top, Some(shallow), 0).unwrap_err();
    assert!(too_deep(&e), "{e:?}");
}

#[test]
fn adding_a_group_next_to_a_layer_in_the_deepest_group_is_refused_and_changes_nothing() {
    let (mut d, _) = chain(MAX_GROUP_DEPTH);
    let leaf = d.layers()[0].id();
    assert_eq!(d.depth_of(leaf).unwrap(), MAX_GROUP_DEPTH);
    let (layers, revision) = (d.layers().len(), d.revision());
    // 一番内側のグループの中のレイヤーの隣へ足すと、グループが 65 段になる
    let e = d.add_group("one more", Some(leaf)).unwrap_err();
    assert!(too_deep(&e), "{e:?}");
    assert_eq!(
        (d.layers().len(), d.revision(), d.undo_count()),
        (layers, revision, 0)
    );
    d.validate_structure().unwrap();
    // 一番上の段・別の段の隣へなら足せる（鎖は増えない）
    d.add_group("top", None).unwrap();
    let top = d.layers().last().unwrap().id();
    d.add_group("above the outer group", Some(top)).unwrap();
    d.validate_structure().unwrap();
}

#[test]
fn adding_a_group_one_below_the_limit_is_allowed() {
    // 鎖が 63 段の一番内側のレイヤーの隣へは、ちょうど 64 段になるので足せる
    let (mut d, _) = chain(MAX_GROUP_DEPTH - 1);
    let leaf = d.layers()[0].id();
    let g = d.add_group("last allowed", Some(leaf)).unwrap();
    assert_eq!(d.depth_of(g).unwrap(), MAX_GROUP_DEPTH - 1);
    d.validate_structure().unwrap();
    // その中のレイヤーの隣へはもう足せない
    let inner = d.add_layer("inner").unwrap();
    d.move_layer_to(inner, Some(g), 0).unwrap();
    let e = d.add_group("too deep", Some(inner)).unwrap_err();
    assert!(too_deep(&e), "{e:?}");
    // 取り消すと、足す前に戻って、また足せる
    d.undo().unwrap(); // 動かす
    d.undo().unwrap(); // レイヤーを足す
    d.undo().unwrap(); // グループを足す
    d.add_group("again", Some(leaf)).unwrap();
}

#[test]
fn undo_back_into_a_valid_nesting_still_works_after_a_refusal() {
    let (mut d, top) = chain(MAX_GROUP_DEPTH - 1);
    let wrapped = d.group_layers(&[top], "last allowed").unwrap();
    assert_eq!(d.depth_of(d.layers()[0].id()).unwrap(), MAX_GROUP_DEPTH);
    assert!(d.group_layers(&[wrapped], "too deep").is_err());
    d.undo().unwrap();
    d.group_layers(&[top], "again").unwrap();
    d.validate_structure().unwrap();
}

/// レイヤーを平らに足し、`parents`（下から上の並び。`None` は一番上の段）を `set_structure_for_load` で渡す。
fn load(
    kinds: &[bool],
    parents: impl Fn(&[LayerId]) -> Vec<Option<LayerId>>,
) -> Result<Document, CoreError> {
    let mut d = doc();
    let ids: Vec<LayerId> = kinds
        .iter()
        .enumerate()
        .map(|(i, &group)| {
            if group {
                d.add_group(&format!("g{i}"), None).unwrap()
            } else {
                d.add_layer(&format!("l{i}")).unwrap()
            }
        })
        .collect();
    let p = parents(&ids);
    d.set_structure_for_load(&p).map(|()| d)
}

/// 並び [leaf, g1, g2, …, gN]（g1 が leaf の親、g2 が g1 の親…）。
fn chain_parents(n: usize) -> Result<Document, CoreError> {
    let kinds: Vec<bool> = std::iter::once(false)
        .chain(std::iter::repeat_n(true, n))
        .collect();
    load(&kinds, |ids| {
        (0..ids.len()).map(|i| ids.get(i + 1).copied()).collect()
    })
}

#[test]
fn loaders_refuse_a_chain_deeper_than_the_limit() {
    chain_parents(MAX_GROUP_DEPTH)
        .unwrap()
        .validate_structure()
        .unwrap();
    let e = chain_parents(MAX_GROUP_DEPTH + 1)
        .err()
        .expect("65 段は断る");
    assert!(too_deep(&e), "{e:?}");
    // 1000 段（取り込みが以前は受けていた深さ）
    let e = chain_parents(1000).err().expect("断る");
    assert!(too_deep(&e), "{e:?}");
}

#[test]
fn nesting_is_checked_in_one_pass_even_for_2000_layers_in_a_deep_chain() {
    // 64 段の鎖で、各グループが 30 枚ほどのラスターを持つ（約 2000 レイヤー）。レイヤーごとに親の鎖をたどって各段で並びを探す確かめは、レイヤーの数 n と
    // 深さ d で n² d に比例して増える作りだが、以前の core の確かめの遅さは測っていない。今の確かめは 1 回なめるだけで、この形（レイヤーの
    // 足し込みを含む）が Linux で dev 約 33ms・release 約 15ms だった。ここの 5 秒は形が崩れたときだけ落ちる大きな余裕
    // （読み手の `nesting_native.rs` は、鎖の各段で間のレイヤーを全部調べる以前の確かめが 13 秒かかった形で、同じ上限を守る）
    let groups = MAX_GROUP_DEPTH;
    let per_group = 30;
    // 並び（下から上）: [g1 の中身の葉 × 30, g1, g2 の葉 × 30, g2, …]。g(k+1) の中身は g(k) と自分の葉
    let mut kinds = Vec::new();
    let mut owner = Vec::new(); // 各レイヤーが入るグループの番号（0 始まり。最後のグループは None）
    for k in 0..groups {
        for _ in 0..per_group {
            kinds.push(false);
            owner.push(Some(k));
        }
        kinds.push(true);
        owner.push(if k + 1 < groups { Some(k + 1) } else { None });
    }
    let n = kinds.len();
    let started = Instant::now();
    let d = load(&kinds, |ids| {
        // グループ k の ID は、その中身の直後にある
        let group_id = |k: usize| ids[(k + 1) * (per_group + 1) - 1];
        owner.iter().map(|o| o.map(group_id)).collect()
    })
    .unwrap();
    d.validate_structure().unwrap();
    let took = started.elapsed();
    assert!(n > 1900, "{n}");
    assert!(
        took < Duration::from_secs(5),
        "{n} レイヤーの入れ子の確かめに {took:?}"
    );
}

#[test]
fn a_document_at_the_depth_limit_composites_in_a_small_thread_stack() {
    // 合成は入れ子を再帰でたどる。上限の深さの文書が、Windows の画面のスレッドと同じ 1MiB のスタックの別スレッドで合成できる
    // （Linux の dev で実測: 64 段は 288KB で溢れ 320KB で収まる。Windows の 1MiB の画面のスレッドでは未確認。上限を上げるときは、
    // この試験の余裕を見直す）
    let (d, _) = chain(MAX_GROUP_DEPTH);
    let handle = std::thread::Builder::new()
        .stack_size(1024 * 1024)
        .spawn(move || d.composite(Rect::new(0, 0, 8, 8)).unwrap().len())
        .unwrap();
    assert_eq!(handle.join().unwrap(), 8 * 8 * 4);
}

#[test]
fn placing_a_smart_material_stops_at_the_depth_limit() {
    // 素材: グループ 1 つの中にラスター 1 枚（グループの鎖は 1 本）
    let mut src = doc();
    let leaf = src.add_layer("葉").unwrap();
    let g = src.group_layers(&[leaf], "g").unwrap();
    let material = src.capture_smart_material(&[g], "素材").unwrap();
    let at = |parent| SmartPlacement {
        parent: Some(parent),
        ..SmartPlacement::default()
    };
    // 63 段の鎖の一番内側へ置くと、ちょうど 64 段
    let (mut d, _) = chain(MAX_GROUP_DEPTH - 1);
    let innermost = d.layers()[1].id();
    d.place_smart_material(&material, &at(innermost)).unwrap();
    d.validate_structure().unwrap();
    // 64 段の鎖の一番内側へは 65 段になるので断り、文書は変わらない
    let (mut d, _) = chain(MAX_GROUP_DEPTH);
    let innermost = d.layers()[1].id();
    let (layers, revision) = (d.layers().len(), d.revision());
    let e = d
        .place_smart_material(&material, &at(innermost))
        .unwrap_err();
    assert!(too_deep(&e), "{e:?}");
    assert_eq!((d.layers().len(), d.revision()), (layers, revision));
    // 一番上の段へは置ける
    d.place_smart_material(&material, &SmartPlacement::default())
        .unwrap();
}
