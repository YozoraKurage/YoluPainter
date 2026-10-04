//! テクスチャセットの焼いたメッシュマップ（種類ごとに最後の 1 枚）。
//!
//! 文書の外の派生物: 描くレイヤーではなく、Undo に入らず、文書の版も変えない。.ylp にはセットごとに `meshmap-<種類>.bin` として
//! 保存する（`project.rs`。焼いて保存していないものだけを書き、開いた時のものはファイルのバイト列のまま残る）。焼き直すと同じ種類を
//! 置き換える。16 bit の正本は `Arc` で持ち（画面・書き出し・保存が同じものを読む）、表示用の 8 bit は持たない。

use std::sync::Arc;

use yolu_core::mesh_maps::{BakedMeshMap, MeshBakeReport, MeshMapKind};
use yolu_gpu::BakeRun;

#[derive(Clone, Debug, Default)]
pub struct MeshMapSet {
    /// 種類の並び（`MeshMapKind::ALL` の順）。
    maps: Vec<Arc<BakedMeshMap>>,
    /// 焼いた後でまだ .ylp に書いていない種類。
    dirty: Vec<MeshMapKind>,
    revision: u64,
    report: Option<MeshBakeReport>,
    run: Option<BakeRun>,
}

impl MeshMapSet {
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }

    pub fn len(&self) -> usize {
        self.maps.len()
    }

    pub fn get(&self, kind: MeshMapKind) -> Option<&Arc<BakedMeshMap>> {
        self.maps.iter().find(|m| m.kind() == kind)
    }

    /// 種類の並びの順。
    pub fn iter(&self) -> impl Iterator<Item = &Arc<BakedMeshMap>> {
        self.maps.iter()
    }

    /// 変わるたびに増える（表示の作り直しの目印）。
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// 今のマップを焼いたベイクの記録（入れたときだけ変わる。取消・時間切れ・捨てた結果では前のまま）。
    pub fn report(&self) -> Option<&MeshBakeReport> {
        self.report.as_ref()
    }

    pub fn set_report(&mut self, report: MeshBakeReport) {
        self.report = Some(report);
    }

    /// 今のマップを焼いた場所（GPU か CPU か、CPU に戻った理由。`report` と同じ入れ方）。
    pub fn run(&self) -> Option<&BakeRun> {
        self.run.as_ref()
    }

    pub fn set_run(&mut self, run: BakeRun) {
        self.run = Some(run);
    }

    fn insert(&mut self, map: BakedMeshMap) {
        let map = Arc::new(map);
        match self.maps.iter().position(|m| m.kind() == map.kind()) {
            Some(i) => self.maps[i] = map,
            None => {
                self.maps.push(map);
                let order = |k: MeshMapKind| MeshMapKind::ALL.iter().position(|a| *a == k);
                self.maps.sort_by_key(|m| order(m.kind()));
            }
        }
    }

    /// 焼いたマップを入れる（同じ種類は置き換える）。まだ保存していない印を付ける。
    pub fn put(&mut self, maps: Vec<BakedMeshMap>) {
        for map in maps {
            let kind = map.kind();
            self.insert(map);
            if !self.dirty.contains(&kind) {
                self.dirty.push(kind);
            }
        }
        self.revision += 1;
    }

    /// .ylp から読んだマップを入れる（保存済みの印）。
    pub fn load(&mut self, map: BakedMeshMap) {
        let kind = map.kind();
        self.insert(map);
        self.dirty.retain(|k| *k != kind);
        self.revision += 1;
    }

    /// まだ .ylp に書いていないマップ（種類の並び）。
    pub fn unsaved(&self) -> Vec<Arc<BakedMeshMap>> {
        self.maps
            .iter()
            .filter(|m| self.dirty.contains(&m.kind()))
            .cloned()
            .collect()
    }

    /// 書いたマップを保存済みにする（書いた後に焼き直されたものは、同じ `Arc` のときだけ）。
    pub fn mark_saved(&mut self, written: &[Arc<BakedMeshMap>]) {
        for w in written {
            let same = self.get(w.kind()).is_some_and(|m| Arc::ptr_eq(m, w));
            if same {
                self.dirty.retain(|k| *k != w.kind());
            }
        }
    }
}
