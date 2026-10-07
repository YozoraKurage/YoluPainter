//! .ylp の正本の読み手が見る入れ子: 親子の確かめはレイヤーの数に対して線形で、グループの入れ子の上限（`yolu_core::MAX_GROUP_DEPTH`）を超える
//! 既存のファイルは、壊れたファイルでなく上限として断る。core の編集は上限を超える入れ子を作らせないので、深い正本は、レイヤーの親の欄を
//! 書き換えたバイト列で作る。

use std::time::{Duration, Instant};
use yolu_core::{Document, MAX_GROUP_DEPTH};
use yolu_io::{Error, NativeDocument, NativeField, NativeValue};

fn write(fields: &[NativeField]) -> Vec<u8> {
    let mut out = Vec::new();
    for f in fields {
        match &f.value {
            NativeValue::Int(v) => out.extend(v.to_le_bytes()),
            NativeValue::Byte(v) => out.push(*v),
            NativeValue::Bool(v) => out.push(u8::from(*v)),
            NativeValue::Float(v) => out.extend(v.to_le_bytes()),
            NativeValue::Guid(v) => out.extend(v),
            NativeValue::Text(v) => {
                out.extend((v.len() as i32).to_le_bytes());
                out.extend(v.as_bytes());
            }
            NativeValue::Bytes(v) => out.extend(v.iter()),
        }
    }
    out
}

/// 平らな文書（レイヤーの種類は `kinds`: true はグループ）の正本。
fn flat(kinds: &[bool]) -> NativeDocument {
    let mut d = Document::with_tile_size(8, 8, 8).unwrap();
    for (i, &group) in kinds.iter().enumerate() {
        if group {
            d.add_group(&format!("g{i}"), None).unwrap();
        } else {
            d.add_layer(&format!("l{i}")).unwrap();
        }
    }
    NativeDocument::from_core(&d).unwrap()
}

/// `layers[i].parent` を `parents[i]`（レイヤーの番号。None は一番上の段）に書き換えたバイト列を読む。
fn read_with_parents(
    native: &NativeDocument,
    parents: &[Option<usize>],
) -> Result<NativeDocument, Error> {
    let mut fields = native.fields().to_vec();
    let ids: Vec<NativeValue> = (0..parents.len())
        .map(|i| native.field(&format!("layers[{i}].id")).unwrap().clone())
        .collect();
    for (i, parent) in parents.iter().enumerate() {
        let path = format!("layers[{i}].parent");
        let value = match parent {
            Some(p) => ids[*p].clone(),
            None => NativeValue::Guid([0; 16]),
        };
        fields.iter_mut().find(|f| f.path == path).unwrap().value = value;
    }
    NativeDocument::read(&write(&fields))
}

/// 並び [葉, g1, g2, …, gN]（g1 が葉の親、g2 が g1 の親…）。
fn chain(n: usize) -> Result<NativeDocument, Error> {
    let kinds: Vec<bool> = std::iter::once(false)
        .chain(std::iter::repeat_n(true, n))
        .collect();
    let parents: Vec<Option<usize>> = (0..kinds.len())
        .map(|i| (i + 1 < kinds.len()).then_some(i + 1))
        .collect();
    read_with_parents(&flat(&kinds), &parents)
}

#[test]
fn the_reader_accepts_a_chain_up_to_the_limit_and_refuses_a_deeper_one_as_a_limit() {
    let ok = chain(MAX_GROUP_DEPTH).unwrap();
    assert_eq!(ok.layer_count(), MAX_GROUP_DEPTH + 1);
    // 読んだ文書は、core の文書にもなる（上限ちょうどは使える）
    assert_eq!(ok.to_core().unwrap().layers().len(), MAX_GROUP_DEPTH + 1);
    for n in [MAX_GROUP_DEPTH + 1, 500] {
        match chain(n) {
            Err(Error::Budget(why)) => assert!(why.contains("入れ子の上限"), "{why}"),
            other => panic!(
                "{n} 段は上限として断る: {:?}",
                other.map(|d| d.layer_count())
            ),
        }
    }
}

#[test]
fn the_parent_rules_still_hold_after_the_one_pass_rewrite() {
    // 正しい: [葉 1・葉 2 ∈ G1, G1 ∈ G2, 葉 3 ∈ G2, G2]
    let kinds = [false, false, true, false, true];
    let native = flat(&kinds);
    let parents = [Some(2), Some(2), Some(4), Some(4), None];
    read_with_parents(&native, &parents).unwrap();
    let invalid = |kinds: &[bool], parents: &[Option<usize>]| match read_with_parents(
        &flat(kinds),
        parents,
    ) {
        Err(Error::InvalidData(_)) => {}
        other => panic!(
            "{parents:?}: 壊れたファイルとして断る: {:?}",
            other.map(|d| d.layer_count())
        ),
    };
    // 親が子より下にある・自分が親・グループでないレイヤーが親
    invalid(&[true, false], &[None, Some(0)]);
    invalid(&[true], &[Some(0)]);
    invalid(&[false, false], &[Some(1), None]);
    // 子が連続していない: [A ∈ G, B（外）, G]
    invalid(&[false, false, true], &[Some(2), None, None]);
    // G1 の中身（A）と G1 の間に、外側の G2 の子（B）が挟まる: [A ∈ G1, B ∈ G2, G1 ∈ G2, G2]
    invalid(
        &[false, false, true, true],
        &[Some(2), Some(3), Some(3), None],
    );
    // 存在しない親
    let native = flat(&[false, true]);
    let mut fields = native.fields().to_vec();
    fields
        .iter_mut()
        .find(|f| f.path == "layers[0].parent")
        .unwrap()
        .value = NativeValue::Guid([7; 16]);
    assert!(matches!(
        NativeDocument::read(&write(&fields)),
        Err(Error::InvalidData(_))
    ));
}

#[test]
fn the_parent_check_reads_2000_layers_in_a_deep_chain_in_one_pass() {
    // 64 段の鎖で、各グループが 30 枚ほどのラスターを持つ（約 2000 レイヤー）。親の鎖をレイヤーごとにたどり、さらに間のレイヤーを全部調べる確かめは、
    // この形で数十億回の比較になり事実上止まる
    let per_group = 30;
    let mut kinds = Vec::new();
    let mut owner: Vec<Option<usize>> = Vec::new();
    for k in 0..MAX_GROUP_DEPTH {
        for _ in 0..per_group {
            kinds.push(false);
            owner.push(Some(k));
        }
        kinds.push(true);
        owner.push((k + 1 < MAX_GROUP_DEPTH).then_some(k + 1));
    }
    let group_index = |k: usize| (k + 1) * (per_group + 1) - 1;
    let parents: Vec<Option<usize>> = owner.iter().map(|o| o.map(group_index)).collect();
    let native = flat(&kinds);
    let started = Instant::now();
    let read = read_with_parents(&native, &parents).unwrap();
    let took = started.elapsed();
    assert!(read.layer_count() > 1900);
    assert!(
        took < Duration::from_secs(5),
        "{} レイヤーの読みに {took:?}",
        read.layer_count()
    );
}
