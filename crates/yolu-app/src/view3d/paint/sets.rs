//! 今のセットでないセットの絵: 兄弟（`sibling`）・辺の上限・同期の要否・縮めて回す（`demote`）。

use super::*;

impl Paint {
    /// 同じ装置・同じツール（ミップのパイプライン・既定の 1 × 1）を使う、まっさらな別の絵（ほかのセット用）。
    pub fn sibling(&self) -> Paint {
        Paint {
            device: self.device.clone(),
            queue: self.queue.clone(),
            mips: self.mips.clone(),
            defaults: self.defaults.clone(),
            set: None,
            version: 0,
            layout_version: 0,
            scratch: Vec::new(),
            scratch_height: Vec::new(),
            budget: self.budget,
            cap: None,
            uncapped: false,
            uid: next_uid(),
            pad_texels: self.pad_texels,
            pad_source: None,
            pad_want: None,
            pad_seen: None,
            pad_rings: None,
            weights: None,
            stats: PaintStats::default(),
        }
    }

    /// 辺の上限を決める（ほかのセットを縮めて持つ。None は GPU の上限だけ）。ゆるめたときは、次の同期が縮めていた絵を元の大きさで
    /// 作り直す。きつくしたときは、`demote` が GPU の中で縮める（できなければ次の同期が作り直す）。
    pub fn set_cap(&mut self, cap: Option<u32>) {
        if self.cap == cap {
            return;
        }
        let looser = match (self.cap, cap) {
            (Some(_), None) => true,
            (Some(old), Some(new)) => new > old,
            (None, _) => false,
        };
        self.uncapped |= looser;
        self.cap = cap;
    }

    /// 文書を最後に同期してから変わっていないか（文書の ID・変化の通し番号・版・Normal の設定が同じ）。同じなら同期は何もしない。
    pub fn synced_with(&self, doc: &Document) -> bool {
        self.set.as_ref().is_some_and(|s| {
            s.doc_id == doc.id()
                && s.serial == doc.change_serial()
                && s.revision == doc.revision()
                && s.normal_settings == doc.normal_settings()
                && (doc.is_coalescing() || s.coarse.iter().all(Vec::is_empty))
        })
    }

    /// 次の同期が絵を全部作り直すか（初め・文書が替わった・縮めを上げる・上限をゆるめた・変化の記録が切れた）。ほかのセットの作り直しは
    /// 1 フレームに数を絞る。
    pub fn needs_rebuild(&self, doc: &Document) -> bool {
        let (want_shift, _) = self.shift_for(doc);
        self.rebuild_needed(doc, want_shift)
    }

    pub(super) fn rebuild_needed(&self, doc: &Document, want_shift: u32) -> bool {
        match &self.set {
            None => true,
            Some(s) => {
                s.doc_id != doc.id()
                    || s.doc_size != (doc.width(), doc.height())
                    // 新しく使い始めたチャンネルで予算を超えるときは、縮めを上げて作り直す（下げるのは文書が替わるとき）
                    || s.shift < want_shift
                    // 上限をゆるめたときは、縮めていた絵を元の大きさへ戻す
                    || (self.uncapped && s.shift > want_shift)
                    // 変化の記録がこの文書のものでない（since がこの文書の番号でない）
                    || s.serial > doc.change_serial()
                    // 塗り広げる UV の形が変わった（モデル・マテリアルが替わった。前の形の塗り広げを残さない）
                    || s.pad != self.pad_want
            }
        }
    }

    /// 同期の作業用のバッファを、容量ごと手放す。合成の作業は文書の大きさ（帯の分、Height から作る Wrap の Normal は文書全体）に
    /// なるので、作り終えたあとも抱えたままだと、ほかのセットの数に比例して CPU のメモリが残る。
    ///
    /// 段の地図（文書の 1 画素 1 バイト）も手放す。その代わり、ほかのセットの文書が 1 タイル変わっただけでも、次の同期で覆いと段の地図を
    /// 文書の解像度で作り直す（UI のスレッドで、取り消せない。4096²・7 万三角形のセット、縮め 2 で、1 タイルの同期が 0.5 ms から
    /// 約 0.1 秒になる。計測の試験 `measure_syncing_another_set_*`）。
    pub fn release_scratch(&mut self) {
        self.scratch = Vec::new();
        self.scratch_height = Vec::new();
        self.pad_rings = None;
    }

    /// 同期の作業用のバッファが抱えているバイト数（容量。段の地図を含む。試験と計測用）。
    pub fn scratch_bytes(&self) -> usize {
        self.scratch.capacity() + self.scratch_height.capacity() + self.padding_bytes()
    }

    /// 上限できつくなった分を、持っている絵のミップの段をコピーして縮める（GPU の中だけ。文書を合成し直さない）。コピーした段は
    /// 文書から縮めて作る段と同じ大きさのときだけ使う（奇数の辺で段の大きさが切り上げと合わないとき・文書が替わっているときは
    /// 何もせず false。次の同期が作り直す）。縮めた絵は、持っていた変化の通し番号のまま。続く同期が、その後に変わったタイルを足す。
    /// コピーは別の積みですぐに出す: 続く同期の書き込み（`write_texture`）は次の `submit` の頭で行われるので、同じ積みの中でコピーすると
    /// 足したタイルを古い絵で上書きしてしまう（`clear_flat_normal` と同じ事情）。
    pub fn demote(&mut self, doc: &Document) -> bool {
        let (want, by_budget) = self.shift_for(doc);
        let Some(set) = self.set.as_ref() else {
            return false;
        };
        if set.doc_id != doc.id()
            || set.doc_size != (doc.width(), doc.height())
            || set.shift >= want
        {
            return false;
        }
        let k = want - set.shift;
        let size = [
            doc.width().div_ceil(1 << want),
            doc.height().div_ceil(1 << want),
        ];
        if k >= set.levels
            || [(set.size[0] >> k).max(1), (set.size[1] >> k).max(1)] != size
            || set.serial > doc.change_serial()
        {
            return false;
        }
        // 粗い絵を見せている間に、重みを使わない作りで書き換えた段・まだ正確に上げ直していない粗いタイルがあるときは、コピーしない（コピーした段 0 に外側の
        // 色が混ざった範囲が残り、離したあとの上げ直しはタイルの範囲しか直さない）。文書から縮めて作り直す
        if set
            .textures
            .iter()
            .flatten()
            .any(|t| t.plain_dirty.iter().any(Option::is_some))
            || set.coarse.iter().any(|c| !c.is_empty())
        {
            return false;
        }
        let (old_levels, old_size) = (set.levels, set.size);
        let levels = old_levels - k;
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("yolu-3d-demote"),
            });
        let mut moved: [Option<ChannelTexture>; 6] = Default::default();
        for slot in Slot::ALL {
            let Some(old) = set.textures[slot.index()].as_ref() else {
                continue;
            };
            let new = self.make_texture(slot.format(), size, levels);
            for j in k..old_levels {
                let (w, h) = ((old_size[0] >> j).max(1), (old_size[1] >> j).max(1));
                encoder.copy_texture_to_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &old.texture,
                        mip_level: j,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyTextureInfo {
                        texture: &new.texture,
                        mip_level: j - k,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                );
            }
            moved[slot.index()] = Some(new);
        }
        self.queue.submit(Some(encoder.finish()));
        // 重みの絵は縮めの段ごとの絵なので、縮めたら次のミップの作り直しで作り直す（ここで作ると、変わっていないセットの段の地図まで作り直す）
        self.weights = None;
        let set = self.set.as_mut().expect("確かめた");
        set.textures = moved;
        set.size = size;
        set.levels = levels;
        set.shift = want;
        set.by_budget = by_budget;
        self.layout_version += 1;
        self.version += 1;
        self.stats.level = want;
        self.stats.by_budget = by_budget;
        self.stats.gpu_bytes = self.gpu_bytes();
        true
    }
}
