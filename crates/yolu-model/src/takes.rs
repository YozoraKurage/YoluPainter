//! FBX のテイク（アニメのスタック）: 一覧と、テイクとフレームからポーズを求める。
//!
//! - 一覧は読み込み（`load_fbx`）のついでに取る。ufbx はアニメを読まない設定（`ignore_animation`）でもスタックの名前・始まり・終わりと、
//!   どの要素のどの値を動かすか（アニメの値のつながり）は読むので、読みの時間とメモリは変わらない（曲線のキーだけを読まない）。
//!   骨の変換か BlendShape の重みを 1 つも動かさないテイクは一覧に入れない。
//! - ポーズは、ファイルを読み直して（形は読まずアニメだけ。`ignore_geometry`）ufbx に評価させる（`evaluate_transform`・
//!   `evaluate_blend_weight`）。曲線は持たない（持つと、全部の骨に毎フレームのキーがあるファイルでは、読んだ後もずっとアニメの分の
//!   メモリを抱える）。キーの間の補間・前後の延長・アニメのレイヤーの重ね・回転の順・ピボット・一様でない継承の補助のノードは、ufbx の評価そのもの。
//! - 読み直したファイルが読んだときと違う形（ノードの名前の並び・BlendShape のチャンネルの名前）なら、当てずに断る（`ModelError::Changed`）。
//!   同じ形なら、ファイルが書き直されていても、今のファイルのテイクを評価する（テイクの並びが変わっていても、名前で引き直す）。
//! - 骨は全部（テイクが動かさない骨はファイルの休みの値）、BlendShape はテイクが動かすチャンネルだけを返す。
//! - 時刻はフレームで指す（ファイルのフレームの速さ。フレーム f の時刻は f ÷ 速さ 秒）。

use std::collections::HashSet;

use yolu_core::skin::BoneTransform;

use crate::fbx::{
    mirror_transform, node_order, parse, read_file, Gate, LoadControl, ModelError, ModelLimits,
    Reading, PARSE_END,
};

/// テイク 1 つ。
#[derive(Clone, Debug, PartialEq)]
pub struct Take {
    pub name: String,
    /// 始まりと終わりのフレーム（ファイルのフレームの速さで丸めた物。終わりは始まり以上）。
    pub first_frame: i64,
    pub last_frame: i64,
    /// ufbx のスタックの番号（読み直したシーンでまず引く。名前が合わなければ名前で探し直す）。
    stack: usize,
}

/// FBX のテイクの一覧と、テイクからポーズを求めるときの対応。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Takes {
    pub list: Vec<Take>,
    /// フレームの速さ（1 秒のフレームの数。ファイルの設定）。
    pub frame_rate: f64,
    /// 読み直したファイルが同じ形かを見る印（ノードの名前の並びと、BlendShape のチャンネルの名前）。
    fingerprint: u64,
    /// `Rig` のメッシュごと・BlendShape ごとの、ufbx のチャンネルの番号。
    channels: Vec<Vec<u32>>,
}

impl Takes {
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    /// 名前でテイクを引く（同じ名前が並べば最初のもの）。
    pub fn find(&self, name: &str) -> Option<usize> {
        self.list.iter().position(|t| t.name == name)
    }
}

/// テイクのフレームのポーズ（読んだ FBX の `Rig` の並び）。
#[derive(Clone, Debug, PartialEq)]
pub struct TakePose {
    /// 骨ごとのローカルの変換（全部の骨。テイクが動かさない骨はファイルの休みの値）。
    pub locals: Vec<BoneTransform>,
    /// テイクが動かす BlendShape の重み（メッシュ・BlendShape・0〜100 の目盛り）。
    pub weights: Vec<(usize, usize, f32)>,
}

/// フレームの数の上限（壊れたファイルの桁外れの長さで、つまみの幅を壊さない）。
const MAX_FRAME: f64 = 1.0e9;

/// 骨の変換を動かす値（ufbx の `evaluate_transform` が読む値）。
const TRANSFORM_PROPS: &[&str] = &[
    "Lcl Translation",
    "Lcl Rotation",
    "Lcl Scaling",
    "PreRotation",
    "PostRotation",
    "RotationOffset",
    "RotationPivot",
    "ScalingOffset",
    "ScalingPivot",
    "RotationOrder",
];

/// BlendShape の重みの値。
const WEIGHT_PROP: &str = "DeformPercent";

/// 文字列の並びの印（FNV-1a。区切りに 0xff を挟む）。
fn fingerprint(scene: &ufbx::Scene, order: &[&ufbx::Node], channels: &[Vec<u32>]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes.iter().chain(&[0xff]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    eat(&(order.len() as u64).to_le_bytes());
    for n in order {
        eat(n.element.name.as_bytes());
    }
    for &c in channels.iter().flatten() {
        match scene.blend_channels.get(c as usize) {
            Some(ch) => eat(ch.element.name.as_bytes()),
            None => eat(b"\0missing"),
        }
    }
    h
}

/// スタックが動かす値（ノードの番号・チャンネルの番号）。
fn animated(stack: &ufbx::AnimStack) -> (HashSet<u32>, HashSet<u32>) {
    let mut nodes = HashSet::new();
    let mut channels = HashSet::new();
    for layer in stack.layers.iter() {
        for p in layer.anim_props.iter() {
            let e = &p.element;
            match e.type_ {
                ufbx::ElementType::Node if TRANSFORM_PROPS.contains(&&*p.prop_name) => {
                    nodes.insert(e.typed_id);
                }
                ufbx::ElementType::BlendChannel if &*p.prop_name == WEIGHT_PROP => {
                    channels.insert(e.typed_id);
                }
                _ => {}
            }
        }
    }
    (nodes, channels)
}

fn frame_rate(scene: &ufbx::Scene) -> f64 {
    let fps = scene.settings.frames_per_second;
    if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        // TimeMode を書かない FBX と同じ（ufbx の既定。試験で確かめている）
        24.0
    }
}

fn frame_of(seconds: f64, fps: f64) -> i64 {
    let f = (seconds * fps).round();
    if f.is_finite() {
        f.clamp(-MAX_FRAME, MAX_FRAME) as i64
    } else {
        0
    }
}

/// スタックが骨の変換か `ours` の BlendShape の重みを動かすか（動かさないスタックはテイクとして一覧に入れない）。
fn moves(stack: &ufbx::AnimStack, order: &[&ufbx::Node], ours: &HashSet<u32>) -> bool {
    let (nodes, weights) = animated(stack);
    // 根は骨の変換を持たない（ufbx の評価も根は休みのまま）
    order
        .iter()
        .any(|n| !n.is_root && nodes.contains(&n.element.typed_id))
        || weights.iter().any(|c| ours.contains(c))
}

/// 読み込みのシーンからテイクの一覧を作る（`channels` は `Rig` の BlendShape ごとの ufbx のチャンネルの番号）。
pub(crate) fn list(scene: &ufbx::Scene, order: &[&ufbx::Node], channels: Vec<Vec<u32>>) -> Takes {
    let fps = frame_rate(scene);
    let ours: HashSet<u32> = channels.iter().flatten().copied().collect();
    let mut out = Vec::new();
    for (i, stack) in scene.anim_stacks.iter().enumerate() {
        if !moves(stack, order, &ours) {
            continue;
        }
        let first = frame_of(stack.time_begin, fps);
        let last = frame_of(stack.time_end, fps).max(first);
        out.push(Take {
            name: stack.element.name.to_string(),
            first_frame: first,
            last_frame: last,
            stack: i,
        });
    }
    Takes {
        list: out,
        frame_rate: fps,
        fingerprint: fingerprint(scene, order, &channels),
        channels,
    }
}

/// テイクのフレームのポーズを、ファイルを読み直して求める（`takes` はそのファイルを `load_fbx` で読んだときの一覧）。
pub fn evaluate_take(
    path: &std::path::Path,
    takes: &Takes,
    take: usize,
    frame: i64,
    limits: &ModelLimits,
) -> Result<TakePose, ModelError> {
    evaluate_take_with(path, takes, take, frame, limits, LoadControl::default())
}

/// `evaluate_take` の、取り消しと進み具合の知らせつきのもの。
pub fn evaluate_take_with(
    path: &std::path::Path,
    takes: &Takes,
    take: usize,
    frame: i64,
    limits: &ModelLimits,
    control: LoadControl<'_>,
) -> Result<TakePose, ModelError> {
    let gate = Gate::new(control);
    gate.check()?;
    gate.report(0.0);
    let data = read_file(path, limits, &gate)?;
    evaluate_with_gate(&data, takes, take, frame, limits, &gate)
}

/// バイト列から（試験・ファイルを持たない呼び手）。
pub fn evaluate_take_bytes(
    data: &[u8],
    takes: &Takes,
    take: usize,
    frame: i64,
    limits: &ModelLimits,
) -> Result<TakePose, ModelError> {
    let gate = Gate::new(LoadControl::default());
    evaluate_with_gate(data, takes, take, frame, limits, &gate)
}

fn evaluate_with_gate(
    data: &[u8],
    takes: &Takes,
    take: usize,
    frame: i64,
    limits: &ModelLimits,
    gate: &Gate<'_>,
) -> Result<TakePose, ModelError> {
    let entry = takes.list.get(take).ok_or(ModelError::NoTake)?;
    let scene = parse(data, limits, Reading::Animation, gate)?;
    gate.check()?;
    gate.report(PARSE_END);
    let order = node_order(&scene);
    if fingerprint(&scene, &order, &takes.channels) != takes.fingerprint {
        return Err(ModelError::Changed);
    }
    // 読み込みのときの番号で引く。書き出し直しでテイクの並びが変わっていたら、同じ名前の最初のテイク（`Takes::find` と同じ引き方）
    let ours: HashSet<u32> = takes.channels.iter().flatten().copied().collect();
    let same_take = |s: &ufbx::AnimStack| *s.element.name == *entry.name && moves(s, &order, &ours);
    let stack = scene
        .anim_stacks
        .get(entry.stack)
        .filter(|s| same_take(s))
        .or_else(|| scene.anim_stacks.iter().find(|s| same_take(s)))
        .ok_or(ModelError::NoTake)?;
    let time = frame as f64 / takes.frame_rate;
    let anim = &stack.anim;
    let locals = order
        .iter()
        .map(|n| mirror_transform(&ufbx::evaluate_transform(anim, n, time)))
        .collect();
    let (_, moved) = animated(stack);
    let mut weights = Vec::new();
    for (m, shapes) in takes.channels.iter().enumerate() {
        for (k, &c) in shapes.iter().enumerate() {
            if !moved.contains(&c) {
                continue;
            }
            let Some(ch) = scene.blend_channels.get(c as usize) else {
                return Err(ModelError::Changed);
            };
            let w = (ufbx::evaluate_blend_weight(anim, ch, time) * 100.0) as f32;
            if w.is_finite() {
                weights.push((m, k, w));
            }
        }
    }
    gate.report(1.0);
    Ok(TakePose { locals, weights })
}
