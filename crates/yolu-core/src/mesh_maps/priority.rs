//! 重なった UV のテクセルをどの三角形が焼くか（ベイクの割り当ての段だけで効く。焼いた後の値の計算は変えない）。
//!
//! 同じテクセルを 2 つ以上の三角形が覆うとき、受け手の並び（`Raster::row`）で先の三角形が持ち主になる。ここはその並びを決める:
//! 手で「焼かない」にした UV アイランドと、0〜1 の外へずらした UV アイランド（選んだときだけ）を受け手から外し、残りを「優先する」に
//! したアイランド → 自動の決め方（番号・3D の面積・ミラーの片側）→ 三角形の番号の順に並べる。既定（番号の小さい方・外さない）は今までと
//! 同じ並び（番号の昇順）で、焼いた値と由来の鍵もバイトまで同じ。
use super::{
    check,
    ids::{find, union},
    input::hash_text,
    MeshBakeInput, Result,
};
use std::collections::{BTreeSet, HashMap};

/// 手で選んだアイランドの数の上限（それぞれの一覧で）。
pub const MAX_OVERLAP_ISLANDS: usize = 4096;

/// 重なったテクセルの持ち主の自動の決め方。
#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MeshOverlapRule {
    /// 番号の小さい三角形（既定）。
    #[default]
    LowestIndex,
    /// 3D の面積の大きい UV アイランド。
    LargerArea,
    /// モデルの空間の +X の側にある UV アイランド（ミラーの片側。重心の x で分ける）。
    PositiveX,
    /// −X の側。
    NegativeX,
}
impl MeshOverlapRule {
    pub const ALL: [Self; 4] = [
        Self::LowestIndex,
        Self::LargerArea,
        Self::PositiveX,
        Self::NegativeX,
    ];
    /// 由来の鍵と記録に書く名前（言語によらない。変えない）。
    pub fn name(self) -> &'static str {
        match self {
            Self::LowestIndex => "LowestIndex",
            Self::LargerArea => "LargerArea",
            Self::PositiveX => "PositiveX",
            Self::NegativeX => "NegativeX",
        }
    }
    pub fn from_index(value: i32) -> Option<Self> {
        Self::ALL.get(usize::try_from(value).ok()?).copied()
    }
}

/// 手で選んだアイランドをどちらの一覧に入れるか。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MeshOverlapList {
    /// 焼かない（受け手から外す。遮蔽には使う）。
    Skip,
    /// 優先する（重なったテクセルを先に取る）。
    Prefer,
}

/// 重なった UV のテクセルの持ち主の決め方（テクスチャセットごと）。アイランドは、そのアイランドの三角形の番号（モデルの全体の通し番号、
/// `MeshBakeInput` の並び）の 1 つで覚え、焼くときに `bake_islands` でアイランドに広げる。番号はモデルの形に結び付くので、結び付けたモデルの
/// 指紋（`MeshBakeInput::topology_hash`）と一緒に持ち、違うモデルでは焼く前に断る。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MeshOverlapPriority {
    pub rule: MeshOverlapRule,
    /// UV の外接矩形が 0〜1 の正方形と重ならないアイランド（0〜1 の外へずらしたアイランド）を焼かない。切っているときは今までどおり、0〜1 の外の
    /// UV があれば焼く前に断る。
    pub skip_outside: bool,
    binding: String,
    skip: BTreeSet<usize>,
    prefer: BTreeSet<usize>,
}
impl MeshOverlapPriority {
    /// 手で選んだアイランドを含めて作る。一覧が空なら指紋は持たない。
    pub fn new(
        rule: MeshOverlapRule,
        skip_outside: bool,
        mut binding: String,
        skip: BTreeSet<usize>,
        prefer: BTreeSet<usize>,
    ) -> Result<Self> {
        if skip.is_empty() && prefer.is_empty() {
            binding.clear();
        }
        let p = Self {
            rule,
            skip_outside,
            binding,
            skip,
            prefer,
        };
        p.validate()?;
        Ok(p)
    }
    pub fn validate(&self) -> Result<()> {
        check(
            self.skip.len() <= MAX_OVERLAP_ISLANDS
                && self.prefer.len() <= MAX_OVERLAP_ISLANDS
                && self.skip.iter().chain(&self.prefer).all(|t| *t < 4_000_000),
            "手で選んだアイランドの数か番号が範囲外です",
        )?;
        check(
            self.skip.is_disjoint(&self.prefer),
            "同じアイランドが「焼かない」と「優先する」の両方にあります",
        )?;
        check(
            (self.skip.is_empty() && self.prefer.is_empty() && self.binding.is_empty())
                || self.binding.len() == 64
                    && self
                        .binding
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "手で選んだアイランドのモデル指紋が不正です",
        )
    }
    /// 今までと同じ（番号の小さい方・外さない・手で選んだアイランドなし）か。
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
    /// 手で選んだアイランドを結び付けたモデルの指紋（一覧が空なら空）。
    pub fn binding(&self) -> &str {
        &self.binding
    }
    /// 「焼かない」のアイランド（アイランドの三角形の番号の 1 つ、昇順）。
    pub fn skipped(&self) -> &BTreeSet<usize> {
        &self.skip
    }
    /// 「優先する」のアイランド。
    pub fn preferred(&self) -> &BTreeSet<usize> {
        &self.prefer
    }
    /// アイランド（その三角形の番号 `island`）をどちらかの一覧に入れる（もう一方からは外す）か、`None` なら両方から外す。別のモデルの一覧に
    /// 足すのは断る（外すのは指紋によらず受ける）。
    pub fn with_island(
        &self,
        binding: &str,
        island: usize,
        list: Option<MeshOverlapList>,
    ) -> Result<Self> {
        let mut skip = self.skip.clone();
        let mut prefer = self.prefer.clone();
        skip.remove(&island);
        prefer.remove(&island);
        let binding = match list {
            None => self.binding.clone(),
            Some(list) => {
                check(
                    (self.skip.is_empty() && self.prefer.is_empty()) || self.binding == binding,
                    "手で選んだアイランドが別のモデルに属しています",
                )?;
                match list {
                    MeshOverlapList::Skip => skip.insert(island),
                    MeshOverlapList::Prefer => prefer.insert(island),
                };
                binding.to_owned()
            }
        };
        Self::new(self.rule, self.skip_outside, binding, skip, prefer)
    }
    /// 由来の鍵（`MeshBakeSettings::kind_key`）に足す文字列。既定なら空（今までの鍵のまま）。
    pub fn key(&self) -> String {
        let mut out = String::new();
        if self.rule != MeshOverlapRule::LowestIndex {
            out.push_str(&format!(";owner={}", self.rule.name()));
        }
        if self.skip_outside {
            out.push_str(";outside=skip");
        }
        if !self.skip.is_empty() || !self.prefer.is_empty() {
            let list = |set: &BTreeSet<usize>| {
                set.iter()
                    .map(|t| t.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            };
            out.push_str(&format!(
                ";islands={}",
                hash_text(&format!(
                    "{};skip={};prefer={}",
                    self.binding,
                    list(&self.skip),
                    list(&self.prefer)
                ))
            ));
        }
        out
    }
}

/// 優先・焼かないの単位のアイランド（三角形ごとの番号。番号はアイランドの一番小さい三角形の順に 0 から）: UV でも 3D の位置でも同じ辺を共有して
/// つながる三角形（同じスロットの中。UV は 1e-6、位置は 1e-5 に量子化した頂点で比べる）。範囲のツールの UV アイランド（UV だけで
/// つなぐ）と違い、UV がぴったり重なったミラーの両側は別のアイランドになる（両側を 1 つのアイランドにすると、片側だけを選べない）。UV の継ぎ目では
/// UV アイランドと同じく分かれる。
pub fn bake_islands(input: &MeshBakeInput) -> Vec<usize> {
    islands_of(&input.corners, &input.uvs, &input.slots)
}

/// `bake_islands` と同じアイランドを、三角形ごとの角の位置（9）・UV（6）・スロット（1）の並びから作る（モデルの表示の形からも同じ決まりで
/// 引けるように）。並びの長さが合わなければ、短い方の三角形の数まで。
pub fn islands_of(corners: &[f32], uvs: &[f32], slots: &[i32]) -> Vec<usize> {
    let n = slots.len().min(corners.len() / 9).min(uvs.len() / 6);
    let mut vertices: HashMap<[i64; 6], usize> = HashMap::new();
    let mut ids = vec![0usize; n * 3];
    for (i, id) in ids.iter_mut().enumerate() {
        let q = |v: f32, scale: f64| (v as f64 * scale).round_ties_even() as i64;
        let key = [
            slots[i / 3] as i64,
            q(uvs[i * 2], 1e6),
            q(uvs[i * 2 + 1], 1e6),
            q(corners[i * 3], 1e5),
            q(corners[i * 3 + 1], 1e5),
            q(corners[i * 3 + 2], 1e5),
        ];
        let next = vertices.len();
        *id = *vertices.entry(key).or_insert(next);
    }
    let mut parent: Vec<usize> = (0..n).collect();
    let mut edges: HashMap<(usize, usize), usize> = HashMap::new();
    for t in 0..n {
        for e in 0..3 {
            let a = ids[t * 3 + e];
            let b = ids[t * 3 + (e + 1) % 3];
            if let Some(other) = edges.insert((a.min(b), a.max(b)), t) {
                union(&mut parent, t, other);
            }
        }
    }
    let mut ranks = HashMap::new();
    (0..n)
        .map(|t| {
            let root = find(&mut parent, t);
            let next = ranks.len();
            *ranks.entry(root).or_insert(next)
        })
        .collect()
}

/// アイランドごとの集計（受け手の三角形だけ）。
#[derive(Clone, Copy, Default)]
struct Island {
    area: f64,
    /// 面積で重みを付けた重心の x の和（`area` で割ると重心）。
    x: f64,
    min: [f64; 2],
    max: [f64; 2],
    seen: bool,
}

fn triangle_area_and_x(input: &MeshBakeInput, t: usize) -> (f64, f64) {
    let p =
        |i: usize| -> [f64; 3] { std::array::from_fn(|a| input.corners[t * 9 + i * 3 + a] as f64) };
    let (a, b, c) = (p(0), p(1), p(2));
    let e1 = super::input::sub(b, a);
    let e2 = super::input::sub(c, a);
    let area = 0.5 * super::input::length(super::input::cross(e1, e2));
    (area, (a[0] + b[0] + c[0]) / 3.)
}

/// 受け手（`candidates`。元の番号の昇順）から焼かないアイランドを外し、重なったテクセルを先に取る順に並べる。0〜1 の外の UV の拒否もここ
/// （外へずらしたアイランドを外したあとの残りで見る）。
pub(crate) fn arrange(
    input: &MeshBakeInput,
    p: &MeshOverlapPriority,
    candidates: Vec<usize>,
) -> Result<Vec<usize>> {
    p.validate()?;
    let manual = !p.skip.is_empty() || !p.prefer.is_empty();
    if manual {
        check(
            p.binding == input.topology_hash,
            "手で選んだアイランドが別のモデルに属しています",
        )?;
        check(
            p.skip
                .iter()
                .chain(&p.prefer)
                .all(|t| *t < input.triangle_count()),
            "手で選んだアイランドの番号がモデルにありません",
        )?;
    }
    let needs_islands = manual || p.skip_outside || p.rule != MeshOverlapRule::LowestIndex;
    if !needs_islands {
        for &t in &candidates {
            check_unit(input, t)?;
        }
        return Ok(candidates);
    }
    let island_of = bake_islands(input);
    let count = island_of.iter().max().map_or(0, |m| m + 1);
    let mut islands = vec![Island::default(); count];
    for &t in &candidates {
        let i = &mut islands[island_of[t]];
        if !i.seen {
            i.min = [f64::MAX; 2];
            i.max = [f64::MIN; 2];
            i.seen = true;
        }
        let (area, x) = triangle_area_and_x(input, t);
        i.area += area;
        i.x += area * x;
        for k in 0..3 {
            for a in 0..2 {
                let v = input.uvs[t * 6 + k * 2 + a] as f64;
                i.min[a] = i.min[a].min(v);
                i.max[a] = i.max[a].max(v);
            }
        }
    }
    let skip: BTreeSet<usize> = p.skip.iter().map(|t| island_of[*t]).collect();
    let prefer: BTreeSet<usize> = p.prefer.iter().map(|t| island_of[*t]).collect();
    // UV の外接矩形が 0〜1 の正方形と（拒否と同じ許し幅を超えて）重ならないアイランド
    const EPS: f64 = 1e-6;
    let outside = |i: &Island| {
        p.skip_outside
            && (i.max[0] <= EPS || i.min[0] >= 1. - EPS || i.max[1] <= EPS || i.min[1] >= 1. - EPS)
    };
    let mut receivers: Vec<usize> = candidates
        .into_iter()
        .filter(|t| {
            let i = island_of[*t];
            !skip.contains(&i) && !outside(&islands[i])
        })
        .collect();
    for &t in &receivers {
        check_unit(input, t)?;
    }
    if p.rule == MeshOverlapRule::LowestIndex && prefer.is_empty() {
        return Ok(receivers);
    }
    // 面積は全体の面積との比を 1e9 段に丸めて比べる（足し算の順だけで生じる差で、同じ形のアイランドの順が入れ替わらないように）
    let total: f64 = islands
        .iter()
        .map(|i| i.area)
        .sum::<f64>()
        .max(f64::MIN_POSITIVE);
    let eps_x = input.diagonal * 1e-6;
    let side = |i: &Island| -> u8 {
        let x = if i.area > 0. { i.x / i.area } else { 0. };
        if x > eps_x {
            0
        } else if x < -eps_x {
            2
        } else {
            1
        }
    };
    let rank = |t: usize| -> (u8, i64, usize) {
        let island = island_of[t];
        let i = &islands[island];
        let preferred = u8::from(!prefer.contains(&island));
        let key = match p.rule {
            MeshOverlapRule::LowestIndex => 0,
            MeshOverlapRule::LargerArea => -((i.area / total * 1e9).round() as i64),
            MeshOverlapRule::PositiveX => side(i) as i64,
            MeshOverlapRule::NegativeX => 2 - side(i) as i64,
        };
        (preferred, key, t)
    };
    receivers.sort_by_cached_key(|t| rank(*t));
    Ok(receivers)
}

fn check_unit(input: &MeshBakeInput, t: usize) -> Result<()> {
    check(
        input.uvs[t * 6..t * 6 + 6]
            .iter()
            .all(|v| *v >= -1e-6f32 && *v <= 1. + 1e-6f32),
        "UVが0〜1の外です。繰り返し・UDIMのUVはベイクできません",
    )
}
