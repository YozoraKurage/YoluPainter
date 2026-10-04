//! 3D の面のクローンの元（Alt を押したクリックで決める、モデルの面の上の点）と、先の基準・揃え方・全レイヤーから読むか。
//! 元は、決めたときのモデルの当たり（世代つき）。モデルが作り直された（ポーズ・隠すテクスチャセットの変更・差し替え）らもう使えない。

use yolu_core::geometry::{SurfaceGeometry, SurfaceHit};

/// クローンの元と、その設定。
#[derive(Clone, Debug, PartialEq)]
pub struct CloneState {
    /// 写し元の面の点（決めていなければ None）。
    pub source: Option<SurfaceHit>,
    /// 揃える: 前のストロークの先の基準の点（次のストロークの最初の点をこれに対応させる）。
    pub destination: Option<SurfaceHit>,
    /// 揃える（前のストロークと同じ位置関係で続ける）。切ると、ストロークごとに最初の点が元に重なる。
    pub aligned: bool,
    /// 描くレイヤーだけでなく、見えているレイヤーの重なりを読む。
    pub all_layers: bool,
}

impl Default for CloneState {
    fn default() -> Self {
        CloneState {
            source: None,
            destination: None,
            aligned: true,
            all_layers: false,
        }
    }
}

impl CloneState {
    /// 元が今のモデルの面として使えるか（世代が同じ）。
    pub fn source_for(&self, geometry: &SurfaceGeometry) -> Option<SurfaceHit> {
        self.source.filter(|s| {
            s.revision == geometry.revision() && (s.triangle as usize) < geometry.triangle_count()
        })
    }

    /// 次のストロークの先の基準（揃えるときだけ。前のストロークと同じモデルのもの）。
    pub fn destination_for(&self, geometry: &SurfaceGeometry) -> Option<SurfaceHit> {
        if !self.aligned {
            return None;
        }
        self.destination.filter(|d| {
            d.revision == geometry.revision() && (d.triangle as usize) < geometry.triangle_count()
        })
    }

    /// 元を決める（先の基準は決め直す）。
    pub fn set_source(&mut self, hit: SurfaceHit) {
        self.source = Some(hit);
        self.destination = None;
    }

    /// 揃えるを切り替える（入れ直したら、前の先の基準を捨てて、次のストロークから揃え直す）。
    pub fn set_aligned(&mut self, aligned: bool) {
        self.aligned = aligned;
        self.destination = None;
    }
}
