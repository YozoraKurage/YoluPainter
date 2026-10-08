//! UV の外への塗り広げの元: モデルの UV の形の要約と、段の地図（`padding::Rings`）の覚え。

use super::*;

impl Paint {
    /// 表示の写しを塗り広げる元を決める（同期の前に毎回。None・UV の三角形が無いマテリアルは塗り広げない）。形が前と違えば、
    /// 次の同期が全部を作り直す（`needs_rebuild`）。形の要約は、面の世代かマテリアルが変わったときだけ数え直す。
    pub fn set_padding(&mut self, source: Option<UvSource>) {
        let source = source.filter(|_| self.pad_texels > 0);
        self.pad_want = source.as_ref().and_then(|uv| {
            let revision = uv.geometry.revision();
            match self.pad_seen {
                Some((r, m, shape)) if r == revision && m == uv.material => shape,
                _ => {
                    let shape = pad_shape(&uv.geometry, uv.material);
                    self.pad_seen = Some((revision, uv.material, shape));
                    shape
                }
            }
        });
        self.pad_source = source;
    }

    /// 試験・計測用: 表示の写しを塗り広げる幅（表示のテクセル。0 は塗り広げない前の仕事）。次の `set_padding` から効く（幅を変えた
    /// だけでは作り直さない。作ってある絵を塗り広げ直すなら `invalidate`）。
    pub fn set_padding_texels(&mut self, texels: u32) {
        self.pad_texels = texels;
    }

    /// 覚えている段の地図のバイト数（試験と計測用）。
    pub fn padding_bytes(&self) -> usize {
        self.pad_rings
            .as_ref()
            .and_then(|p| p.rings.as_ref())
            .map_or(0, |r| r.bytes())
    }

    /// 今の絵を塗り広げた形の段の地図（無ければ作る。形・文書の大きさ・縮めが同じなら覚えたもの）。塗り広げないなら None。
    pub(super) fn rings(&mut self, doc: &Document) -> Option<Arc<Rings>> {
        let set = self.set.as_ref()?;
        let shape = set.pad?;
        let reach = (self.pad_texels << set.shift).min(padding::MAX_RING_REACH);
        let size = (doc.width(), doc.height());
        let fresh = self
            .pad_rings
            .as_ref()
            .is_some_and(|p| p.shape == shape && p.size == size && p.reach == reach);
        if !fresh {
            let uv = self
                .pad_source
                .as_ref()
                .filter(|_| self.pad_want == Some(shape))?;
            let (w, h) = (size.0 as f64, size.1 as f64);
            let at = |v: yolu_core::glam::Vec2| DVec2::new(v.x as f64 * w, v.y as f64 * h);
            let triangles = uv
                .geometry
                .triangles()
                .iter()
                .filter(|t| t.material == uv.material)
                .map(|t| [at(t.uv_a), at(t.uv_b), at(t.uv_c)]);
            let rings = padding::coverage(size.0, size.1, triangles)
                .ok()
                .filter(|keep| keep.contains(&true))
                .and_then(|keep| Rings::new(size.0, size.1, &keep, reach).ok())
                .map(Arc::new);
            self.pad_rings = Some(PadRings {
                shape,
                size,
                reach,
                rings,
            });
        }
        self.pad_rings.as_ref().and_then(|p| p.rings.clone())
    }
}

/// マテリアルの三角形の UV の形の要約（UV の値のビットと三角形の並び）。三角形が無ければ None（塗り広げない）。
pub(super) fn pad_shape(geometry: &SurfaceGeometry, material: i32) -> Option<PadShape> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    let mut triangles = 0usize;
    for t in geometry
        .triangles()
        .iter()
        .filter(|t| t.material == material)
    {
        for v in [t.uv_a, t.uv_b, t.uv_c] {
            v.x.to_bits().hash(&mut hasher);
            v.y.to_bits().hash(&mut hasher);
        }
        triangles += 1;
    }
    (triangles > 0).then(|| PadShape {
        hash: hasher.finish(),
        triangles,
    })
}
