use super::{check, input::hash_text, MeshBakeInput, MeshIdSource, Result};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[derive(Clone, Debug, Default)]
pub struct IdColorAssignments {
    pub(crate) binding: String,
    pub(crate) colors: BTreeMap<usize, u32>,
}
impl IdColorAssignments {
    pub fn new(mut binding: String, colors: BTreeMap<usize, u32>) -> Result<Self> {
        if colors.is_empty() {
            binding.clear();
        }
        check(
            colors.len() <= 4096 && colors.iter().all(|(k, v)| *k < 4_000_000 && *v <= 0xffffff),
            "手動ID色の数・番号・色が範囲外です",
        )?;
        check(
            colors.is_empty()
                || binding.len() == 64
                    && binding
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "手動ID色のモデル指紋が不正です",
        )?;
        Ok(Self { binding, colors })
    }
    pub fn key(&self) -> String {
        if self.colors.is_empty() {
            String::new()
        } else {
            hash_text(&format!(
                "{};{}",
                self.binding,
                self.colors
                    .iter()
                    .map(|(k, v)| format!("{k}={v:06X}"))
                    .collect::<Vec<_>>()
                    .join(";")
            ))
        }
    }
}
pub(crate) fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}
pub(crate) fn union(parent: &mut [usize], a: usize, b: usize) {
    let a = find(parent, a);
    let b = find(parent, b);
    parent[a.max(b)] = a.min(b);
}
pub fn mesh_regions(input: &MeshBakeInput, uv: bool) -> Vec<usize> {
    let mut vertices = HashMap::new();
    let mut edges = HashMap::new();
    let mut parent: Vec<_> = (0..input.triangle_count()).collect();
    let mut ids = vec![0; input.triangle_count() * 3];
    for (i, id) in ids.iter_mut().enumerate() {
        let mut key = [0i64; 4];
        key[0] = input.slots[i / 3] as i64;
        let (values, dim, q) = if uv {
            (&input.uvs, 2, 1e6)
        } else {
            (&input.corners, 3, 1e5)
        };
        for a in 0..dim {
            key[a + 1] = (values[i * dim + a] as f64 * q).round_ties_even() as i64;
        }
        let next = vertices.len();
        *id = *vertices.entry(key).or_insert(next);
    }
    for t in 0..input.triangle_count() {
        for e in 0..3 {
            let a = ids[t * 3 + e];
            let b = ids[t * 3 + (e + 1) % 3];
            let key = (a.min(b), a.max(b));
            if let Some(other) = edges.insert(key, t) {
                union(&mut parent, t, other);
            }
        }
    }
    let mut ranks = HashMap::new();
    (0..input.triangle_count())
        .map(|i| {
            let root = find(&mut parent, i);
            let n = ranks.len();
            *ranks.entry(root).or_insert(n)
        })
        .collect()
}
pub fn id_part_binding(input: &MeshBakeInput) -> (Vec<usize>, String) {
    let parts = mesh_regions(input, false);
    let mut h = Sha256::new();
    h.update(input.topology_hash.as_bytes());
    for &p in &parts {
        h.update((p as i32).to_le_bytes());
    }
    (parts, format!("{:x}", h.finalize()))
}
/// count 色を収める格子の段の数 q（2〜256）。
pub fn id_palette_levels(count: usize) -> Result<usize> {
    (2usize..=256)
        .find(|q| q * q * q - q >= count)
        .ok_or_else(|| super::MeshMapError("IDの部品が多すぎます".into()))
}
fn palette_level(i: usize, q: usize) -> u32 {
    ((i * 255 + (q - 1) / 2) / (q - 1)) as u32
}
/// count 色のどの 2 色も、どれかのチャンネルでこれ以上違う（8 bit）。
pub fn id_palette_separation(count: usize) -> Result<u32> {
    let q = id_palette_levels(count)?;
    Ok((0..q - 1)
        .map(|i| palette_level(i + 1, q) - palette_level(i, q))
        .min()
        .unwrap_or(255)
        .min(255))
}
pub fn id_palette(count: usize) -> Result<Vec<u32>> {
    let q = id_palette_levels(count)?;
    let block = q * q + q + 1;
    let candidates = q * q * q - q;
    let mut stride = (candidates as f64 * 0.38196601125010515)
        .round_ties_even()
        .max(1.) as usize;
    fn gcd(mut a: usize, mut b: usize) -> usize {
        while b != 0 {
            (a, b) = (b, a % b);
        }
        a
    }
    while gcd(stride, candidates) != 1 {
        stride += 1;
    }
    Ok((0..count)
        .map(|o| {
            let m = o * stride % candidates;
            let c = m / (block - 1) * block + 1 + m % (block - 1);
            palette_level(c / (q * q), q) << 16
                | palette_level(c / q % q, q) << 8
                | palette_level(c % q, q)
        })
        .collect())
}
pub(crate) struct IdTable {
    pub low: Vec<u32>,
    pub high: Vec<u32>,
    pub manual: Vec<Option<u32>>,
    pub parts: usize,
}
impl IdTable {
    pub fn new(
        low: &MeshBakeInput,
        high: Option<&MeshBakeInput>,
        source: MeshIdSource,
        receivers: &[usize],
        manual: &IdColorAssignments,
    ) -> Result<Self> {
        let mut result = Self {
            low: vec![0xffffff; low.triangle_count()],
            high: vec![0xffffff; high.map_or(0, |h| h.triangle_count())],
            manual: vec![],
            parts: 0,
        };
        if !manual.colors.is_empty() {
            let (parts, binding) = id_part_binding(low);
            let count = parts.iter().max().copied().unwrap_or(0) + 1;
            check(
                binding == manual.binding && manual.colors.keys().all(|p| *p < count),
                "手動ID色が別のモデルまたは分割に属しています",
            )?;
            result.manual = parts
                .iter()
                .map(|p| manual.colors.get(p).copied())
                .collect();
        }
        if source == MeshIdSource::VertexColor {
            fn colors(input: &MeshBakeInput, out: &mut [u32]) {
                if let Some(colors) = &input.colors {
                    for (t, dest) in out.iter_mut().enumerate() {
                        let rgb = |corner: usize| {
                            let mut rgb = 0;
                            for c in 0..3 {
                                let v = colors[t * 12 + corner * 4 + c];
                                let value = if v <= 0. {
                                    0
                                } else if v >= 1. {
                                    255
                                } else {
                                    ((v * 255.) as f64 + 0.5) as u32
                                };
                                rgb = rgb << 8 | value;
                            }
                            rgb
                        };
                        let a = rgb(0);
                        let b = rgb(1);
                        let c = rgb(2);
                        *dest = if a == b || a == c {
                            a
                        } else if b == c {
                            b
                        } else {
                            a.min(b.min(c))
                        };
                    }
                }
            }
            colors(low, &mut result.low);
            if let Some(h) = high {
                colors(h, &mut result.high);
            }
            return Ok(result);
        }
        if source == MeshIdSource::MaterialAsset {
            fn key(i: &MeshBakeInput, t: usize, high: bool) -> String {
                match i
                    .material_keys
                    .as_ref()
                    .and_then(|k| k[t].as_deref())
                    .filter(|s| !s.is_empty())
                {
                    Some(k) => format!("asset:{k}"),
                    None => format!("{}:{}", if high { "high" } else { "low" }, i.slots[t]),
                }
            }
            let mut keys = BTreeSet::new();
            for t in 0..low.triangle_count() {
                if low.slots[t] >= 0 {
                    keys.insert(key(low, t, false));
                }
            }
            if let Some(h) = high {
                for t in 0..h.triangle_count() {
                    keys.insert(key(h, t, true));
                }
            }
            // .NET Ordinal は UTF-16 の符号なし順。
            let mut keys: Vec<_> = keys.into_iter().collect();
            keys.sort_by_cached_key(|s| s.encode_utf16().collect::<Vec<_>>());
            result.parts = keys.len();
            let palette = id_palette(keys.len())?;
            let by_key: HashMap<_, _> = keys.into_iter().zip(palette).collect();
            for &t in receivers {
                result.low[t] = by_key[&key(low, t, false)];
            }
            if let Some(h) = high {
                for t in 0..h.triangle_count() {
                    result.high[t] = by_key[&key(h, t, true)];
                }
            }
            return Ok(result);
        }
        fn keys(i: &MeshBakeInput, s: MeshIdSource) -> Vec<i32> {
            match s {
                MeshIdSource::MaterialSlot => i.slots.clone(),
                MeshIdSource::Mesh => i.renderers.clone(),
                _ => mesh_regions(i, s == MeshIdSource::UvIsland)
                    .into_iter()
                    .map(|v| v as i32)
                    .collect(),
            }
        }
        let lk = keys(low, source);
        let lr: BTreeSet<_> = receivers.iter().map(|t| lk[*t]).collect();
        let lr: HashMap<_, _> = lr.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
        let hk = high
            .filter(|_| source != MeshIdSource::UvIsland)
            .map(|i| keys(i, source));
        let hr: BTreeSet<_> = hk.iter().flatten().copied().collect();
        let hr: HashMap<_, _> = hr.into_iter().enumerate().map(|(i, k)| (k, i)).collect();
        result.parts = lr.len() + hr.len();
        let palette = id_palette(result.parts)?;
        for &t in receivers {
            result.low[t] = palette[lr[&lk[t]]];
        }
        if let Some(hk) = hk {
            for (t, k) in hk.iter().enumerate() {
                result.high[t] = palette[lr.len() + hr[k]];
            }
        }
        Ok(result)
    }
}
