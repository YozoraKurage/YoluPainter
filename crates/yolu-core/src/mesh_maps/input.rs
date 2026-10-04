use super::{check, Result};
use sha2::{Digest, Sha256};

/// 三角形ごとに独立した角。構築時に検証し、その後は読み取り専用。
#[derive(Clone, Debug)]
pub struct MeshBakeInput {
    pub(crate) corners: Vec<f32>,
    pub(crate) normals: Option<Vec<f32>>,
    pub(crate) uvs: Vec<f32>,
    pub(crate) slots: Vec<i32>,
    pub(crate) tangents: Option<Vec<f32>>,
    pub(crate) colors: Option<Vec<f32>>,
    pub(crate) renderers: Vec<i32>,
    pub(crate) renderer_names: Option<Vec<String>>,
    pub(crate) material_keys: Option<Vec<Option<String>>>,
    pub(crate) min: [f64; 3],
    pub(crate) max: [f64; 3],
    pub(crate) diagonal: f64,
    pub(crate) uv_channel: i32,
    pub(crate) normal_source: String,
    pub(crate) hash: String,
    pub(crate) topology_hash: String,
    pub(crate) material_identity_hash: String,
}
#[derive(Clone, Debug, Default)]
pub struct MeshBakeAttributes {
    pub normals: Option<Vec<f32>>,
    pub tangents: Option<Vec<f32>>,
    pub colors: Option<Vec<f32>>,
    pub renderers: Option<Vec<i32>>,
    pub renderer_names: Option<Vec<String>>,
    pub material_keys: Option<Vec<Option<String>>>,
    pub uv_channel: i32,
    pub normal_source: Option<String>,
}
pub(crate) fn hash_text(s: &str) -> String {
    format!("{:x}", Sha256::digest(s.as_bytes()))
}
impl MeshBakeInput {
    pub fn new(
        corners: Vec<f32>,
        uvs: Vec<f32>,
        slots: Vec<i32>,
        attrs: MeshBakeAttributes,
    ) -> Result<Self> {
        let n = corners.len() / 9;
        check(
            corners.len().is_multiple_of(9) && n > 0 && n <= 4_000_000,
            "三角形の角は9成分ずつ、1〜4000000面が必要です",
        )?;
        check(
            uvs.len() == n * 6 && slots.len() == n,
            "UV またはスロットの数が面数と一致しません",
        )?;
        for (values, size) in [
            (&attrs.normals, n * 9),
            (&attrs.tangents, n * 12),
            (&attrs.colors, n * 12),
        ] {
            check(
                values
                    .as_ref()
                    .is_none_or(|v| v.len() == size && v.iter().all(|f| f.is_finite())),
                "頂点属性の数または値が不正です",
            )?;
        }
        check(
            corners.iter().chain(&uvs).all(|v| v.is_finite()),
            "位置・UV に有限でない値があります",
        )?;
        check(
            slots.iter().all(|s| *s >= -1) && (0..=7).contains(&attrs.uv_channel),
            "スロットまたは UV チャンネルが範囲外です",
        )?;
        check(
            attrs.renderers.as_ref().is_none_or(|r| {
                r.len() == n
                    && r.iter().all(|r| {
                        *r >= 0
                            && attrs
                                .renderer_names
                                .as_ref()
                                .is_none_or(|names| (*r as usize) < names.len())
                    })
            }),
            "レンダラー番号が不正です",
        )?;
        check(
            attrs.material_keys.as_ref().is_none_or(|k| {
                k.len() == n
                    && k.iter()
                        .all(|s| s.as_ref().is_none_or(|s| s.encode_utf16().count() <= 128))
            }),
            "マテリアル識別子の数または長さが不正です",
        )?;
        let mut min = [f64::MAX; 3];
        let mut max = [f64::MIN; 3];
        for p in corners.chunks_exact(3) {
            for a in 0..3 {
                min[a] = min[a].min(p[a] as f64);
                max[a] = max[a].max(p[a] as f64);
            }
        }
        let d = std::array::from_fn(|a| max[a] - min[a]);
        let diagonal = length(d);
        check(
            diagonal > 0. && diagonal.is_finite(),
            "メッシュの大きさが0または範囲外です",
        )?;
        let normal_source = if attrs.normals.is_none() {
            "face".into()
        } else {
            attrs
                .normal_source
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "authored".into())
        };
        let mut material = String::new();
        if let Some(keys) = &attrs.material_keys {
            for k in keys {
                material.push_str(&format!(
                    "{}:{}",
                    k.as_ref().map_or(-1, |s| s.encode_utf16().count() as i32),
                    k.as_deref().unwrap_or("")
                ));
            }
        }
        let mut result = Self {
            corners,
            uvs,
            slots,
            normals: attrs.normals,
            tangents: attrs.tangents,
            colors: attrs.colors,
            renderers: attrs.renderers.unwrap_or_else(|| vec![0; n]),
            renderer_names: attrs.renderer_names,
            material_keys: attrs.material_keys,
            min,
            max,
            diagonal,
            uv_channel: attrs.uv_channel,
            normal_source,
            hash: String::new(),
            topology_hash: String::new(),
            material_identity_hash: hash_text(&material),
        };
        result.hash = result.compute_hash(true);
        result.topology_hash = result.compute_hash(false);
        Ok(result)
    }
    fn compute_hash(&self, geometry: bool) -> String {
        let mut h = Sha256::new();
        h.update(if geometry {
            "YOLUPAINTER-MESHBAKE-INPUT-2\n"
        } else {
            "YOLUPAINTER-MESHBAKE-TOPOLOGY-2\n"
        });
        h.update((self.triangle_count() as i32).to_le_bytes());
        h.update(self.uv_channel.to_le_bytes());
        if geometry {
            let flags = i32::from(self.normals.is_some())
                | (i32::from(self.tangents.is_some()) << 1)
                | (i32::from(self.colors.is_some()) << 2);
            h.update(flags.to_le_bytes());
            for a in [
                Some(&self.corners),
                self.normals.as_ref(),
                self.tangents.as_ref(),
                self.colors.as_ref(),
            ]
            .into_iter()
            .flatten()
            {
                for f in a {
                    h.update(f.to_le_bytes());
                }
            }
            if let Some(names) = &self.renderer_names {
                for s in names {
                    h.update((s.len() as i32).to_le_bytes());
                    h.update(s.as_bytes());
                }
            }
        }
        for f in &self.uvs {
            h.update(f.to_le_bytes());
        }
        for n in self.slots.iter().chain(&self.renderers) {
            h.update(n.to_le_bytes());
        }
        format!("{:x}", h.finalize())
    }
    pub fn hash(&self) -> &str {
        &self.hash
    }
    pub fn topology_hash(&self) -> &str {
        &self.topology_hash
    }
    pub fn material_identity_hash(&self) -> &str {
        &self.material_identity_hash
    }
    pub fn normal_source(&self) -> &str {
        &self.normal_source
    }
    pub fn uv_channel(&self) -> i32 {
        self.uv_channel
    }
    pub fn triangle_count(&self) -> usize {
        self.slots.len()
    }
    pub fn diagonal(&self) -> f64 {
        self.diagonal
    }
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        (self.min, self.max)
    }
}
pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub(crate) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
