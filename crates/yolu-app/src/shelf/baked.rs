//! アセットに並べる、今のテクスチャセットのベイクしたマップ。
//!
//! プロジェクトの棚（.ylp の resources）には入らない。焼いたマップ（`bake::maps::MeshMapSet`）の一覧を写した表示で、焼き直す・テクスチャセットを
//! 替えると一覧が替わる（焼いていない種類は出さない）。項目の ID は `baked:<種類の名前>`。サムネイルは表示用の色（`BakedMeshMap::to_rgba8` と同じ）を元の大きさに広げずに平均した絵で、
//! 焼き直した（`revision` が替わった）ら作り直す。使い方は 2 つ: 塗りつぶしレイヤーのチャンネルの源にする（`FillOp::BakedMap`。画像に写さず、
//! 焼き直すと結果も追従する）、新しい塗りつぶしレイヤーとして置く（`place_baked_map`）。

use std::collections::HashMap;
use std::sync::{Arc, Weak};

use yolu_core::mesh_maps::{BakedMeshMap, MeshMapKind};
use yolu_io::Resource;

use super::{is_builtin, Block, Inspected, Picture, PlaceTarget, Thumb, Unreadable};
use crate::engine::{ChannelKind, CoreError, Document, LayerId};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;

/// 索引の種類の文字列（`ItemKind::of`）。
pub(super) const KIND: &str = "bakedMap";

/// 焼いたマップの項目の ID の前置き（棚の項目の ID は UUID、同梱の素材は `builtin:` なので重ならない）。
pub const BAKED_PREFIX: &str = "baked:";

/// 焼いたマップの項目か。
pub fn is_baked(id: &str) -> bool {
    id.starts_with(BAKED_PREFIX)
}

/// プロジェクトの棚に入っていない項目か（同梱の素材・焼いたマップ。消す・書き出す・ライブラリへ入れるは断る）。
pub fn is_derived(id: &str) -> bool {
    is_builtin(id) || is_baked(id)
}

/// 種類の項目の ID。
pub fn baked_id(kind: MeshMapKind) -> String {
    format!("{BAKED_PREFIX}{}", kind.name())
}

/// 項目の ID の種類（焼いたマップの項目でなければ None）。
pub fn baked_kind(id: &str) -> Option<MeshMapKind> {
    let name = id.strip_prefix(BAKED_PREFIX)?;
    MeshMapKind::ALL.into_iter().find(|k| k.name() == name)
}

/// 項目の写しの鍵（テクスチャセットと、焼き直しで増える番号）。
type Key = (u32, u64);

/// 棚が持つ、焼いたマップの項目（`ShelfState::sync_baked` が写す）。
#[derive(Default)]
pub(super) struct Baked {
    key: Option<Key>,
    lang: Option<Lang>,
    items: Vec<Resource>,
    /// 持つのは焼いたマップの持ち主（セットのマップの束）と、この写しだけ。サムネイルの仕事へは弱い参照で渡す（焼き直した後の古いマップを
    /// 仕事が持ち続けない）。
    maps: HashMap<String, Arc<BakedMeshMap>>,
}

impl Baked {
    pub(super) fn items(&self) -> &[Resource] {
        &self.items
    }

    pub(super) fn map(&self, id: &str) -> Option<Weak<BakedMeshMap>> {
        self.maps.get(id).map(Arc::downgrade)
    }
}

fn item(kind: MeshMapKind, map: &BakedMeshMap, key: Key, lang: Lang) -> Resource {
    Resource {
        id: baked_id(kind),
        kind: KIND.into(),
        name: crate::bake::kind_label(lang, kind).to_owned(),
        // 焼き直すと変わる（別のスレッドで見た結果が、古い中身の物なら捨てられる）
        content: format!("{}:{}", key.0, key.1),
        entry: String::new(),
        metadata: serde_json::json!({ "width": map.width(), "height": map.height() }),
    }
}

impl super::ShelfState {
    /// 写してある項目が、このテクスチャセット・焼き直しの番号・言語のものか（毎フレームの確かめ。そうなら写し直さない）。
    pub fn baked_is_current(&self, set: u32, revision: u64, lang: Lang) -> bool {
        self.baked.key == Some((set, revision)) && self.baked.lang == Some(lang)
    }

    /// 今のテクスチャセット（`set`）のベイクしたマップ（`revision` は焼き直すと増える。`maps` は種類の並びの順）を項目に写す。同じ状態・同じ言語なら
    /// 何もしない。焼き直した・セットを替えたときは、前の項目のサムネイルを捨てて作り直す（言語だけ替わったときは名前だけ替える）。
    pub fn sync_baked(&mut self, set: u32, revision: u64, maps: &[Arc<BakedMeshMap>], lang: Lang) {
        let key = (set, revision);
        if self.baked_is_current(set, revision, lang) {
            return;
        }
        if self.baked.key == Some(key) {
            for r in &mut self.baked.items {
                if let Some(kind) = baked_kind(&r.id) {
                    r.name = crate::bake::kind_label(lang, kind).to_owned();
                }
            }
            self.baked.lang = Some(lang);
            return;
        }
        for r in &self.baked.items {
            self.inspected.remove(&r.id);
        }
        self.baked.items = maps.iter().map(|m| item(m.kind(), m, key, lang)).collect();
        self.baked.maps = maps
            .iter()
            .map(|m| (baked_id(m.kind()), m.clone()))
            .collect();
        self.baked.key = Some(key);
        self.baked.lang = Some(lang);
    }
}

/// 焼いたマップのサムネイル（表示用の 8 bit にして、長い辺が `THUMB` に収まる大きさへ。覆いの無い画素は透明）。
pub(super) fn inspect(map: Option<&BakedMeshMap>, stopped: impl Fn() -> bool) -> Inspected {
    let Some(map) = map else {
        return Inspected::bare(Some(Block::Unreadable(Unreadable::same("?"))));
    };
    let mut info = Inspected::bare(None);
    info.width = map.width() as u32;
    info.height = map.height() as u32;
    let source = Texels {
        data: map.data(),
        coverage: map.coverage(),
        channels: map.channels(),
        width: map.width(),
        height: map.height(),
    };
    if let Some(picture) = thumb_picture(&source, stopped) {
        info.thumb = Thumb::Pending(picture);
    }
    info
}

/// 元のマップの画素（`BakedMeshMap` の中身をそのまま借りる）。
struct Texels<'a> {
    data: &'a [u16],
    coverage: &'a [u8],
    channels: usize,
    width: usize,
    height: usize,
}

impl Texels<'_> {
    /// 画素 (x, y) の表示用の色（`BakedMeshMap::to_rgba8(false)` と同じ。覆いが無ければ透明）。
    fn rgba(&self, x: usize, y: usize) -> [u32; 4] {
        let i = y * self.width + x;
        if self.coverage[i] == 0 {
            return [0; 4];
        }
        let mut p = [0, 0, 0, 255];
        for (c, v) in p[..3].iter_mut().enumerate() {
            let raw = self.data[i * self.channels + if self.channels == 1 { 0 } else { c }];
            *v = (raw as u32 * 255 + 32767) / 65535;
        }
        p
    }
}

/// サムネイルの画素 `o`（全部で `dst` 画素）が見る、元の画素の範囲。縮めるときは区切りで割った箱（空にしない）、
/// 広げるときは中心の 1 画素（画素のまま見せる）。
fn span(o: usize, src: usize, dst: usize) -> (usize, usize) {
    if dst >= src {
        let s = ((2 * o + 1) * src) / (2 * dst);
        return (s, s + 1);
    }
    let a = o * src / dst;
    (a, ((o + 1) * src / dst).max(a + 1))
}

/// サムネイルの画素ごとの、元の画素の範囲（`xs`: 横、`ys`: 縦）。
struct Plan {
    xs: Vec<(usize, usize)>,
    ys: Vec<(usize, usize)>,
}

impl Plan {
    fn new(width: usize, height: usize) -> Self {
        let (tw, th) = super::fit_size(width as u32, height as u32);
        Plan {
            xs: (0..tw as usize)
                .map(|o| span(o, width, tw as usize))
                .collect(),
            ys: (0..th as usize)
                .map(|o| span(o, height, th as usize))
                .collect(),
        }
    }

    /// 作るサムネイルの画素の数。
    fn pixels(&self) -> usize {
        self.xs.len() * self.ys.len()
    }

    /// 一時に持つバイト数（出力の RGBA8 と、1 行ぶんの合計。元の大きさに比べない）。
    #[cfg(test)]
    fn scratch_bytes(&self) -> usize {
        self.pixels() * 4 + self.xs.len() * std::mem::size_of::<[u32; 4]>()
    }
}

/// 元の大きさの RGBA8 を作らず、サムネイルの 1 行ぶんの合計だけを持って平均する。行ごとに `stopped` を見て、止められたら None。
fn thumb_picture(source: &Texels, stopped: impl Fn() -> bool) -> Option<Picture> {
    if source.width == 0 || source.height == 0 {
        return None;
    }
    let plan = Plan::new(source.width, source.height);
    let (tw, th) = (plan.xs.len(), plan.ys.len());
    let mut rgba = Vec::with_capacity(plan.pixels() * 4);
    let mut sums = vec![[0u32; 4]; tw];
    for &(y0, y1) in &plan.ys {
        if stopped() {
            return None;
        }
        sums.fill([0; 4]);
        for y in y0..y1 {
            for (sum, &(x0, x1)) in sums.iter_mut().zip(&plan.xs) {
                for x in x0..x1 {
                    for (s, v) in sum.iter_mut().zip(source.rgba(x, y)) {
                        *s += v;
                    }
                }
            }
        }
        for (sum, &(x0, x1)) in sums.iter().zip(&plan.xs) {
            let n = ((x1 - x0) * (y1 - y0)) as u32;
            rgba.extend(sum.iter().map(|s| ((s + n / 2) / n) as u8));
        }
    }
    Some(Picture {
        width: tw as u32,
        height: th as u32,
        rgba,
    })
}

impl AppState {
    /// 今のテクスチャセットのベイクしたマップを、アセットの項目へ写す（毎フレーム。変わっていなければ何もしない）。
    pub fn sync_baked_assets(&mut self) {
        let set = self.session.sets.current();
        if self
            .shelf
            .baked_is_current(set.uid, set.mesh_maps.revision(), self.lang)
        {
            return;
        }
        let maps: Vec<Arc<BakedMeshMap>> = set.mesh_maps.iter().cloned().collect();
        self.shelf
            .sync_baked(set.uid, set.mesh_maps.revision(), &maps, self.lang);
    }

    /// 焼いたマップを、新しい塗りつぶしレイヤーの源（描くチャンネル）として置く（1 回の Undo）。`target` は、レイヤーの一覧へ落とした所
    /// （選んでいるレイヤーのすぐ上、または親のグループの中の位置）。まだ焼いていない種類・描くチャンネルが標準でないときは、何も作らず理由を出す。
    pub(crate) fn place_baked_map(&mut self, kind: MeshMapKind, name: &str, target: PlaceTarget) {
        let lang = self.lang;
        if self.is_stroking() {
            return self.refuse(Source::Assets, crate::lang::refusals::during_stroke(lang));
        }
        let channel = self.m2.paint_channel;
        let value = crate::fillfx::default_fallback(
            self.session
                .doc
                .channel_info(channel)
                .map_or(ChannelKind::Color, |c| c.kind),
        );
        let at = match target {
            PlaceTarget::At { parent, position } => Some((parent, position)),
            _ => None,
        };
        let (above, move_to) = self.new_layer_slot();
        self.session.doc.end_coalescing();
        let source = crate::fx::inputs::map_kind(kind);
        let created = self
            .session
            .doc
            .batch_as(yolu_core::HistoryKind::Batch, |d| {
                let id = d.add_fill_layer(name, &[(channel, value)], above)?;
                if let Some(set) = move_to {
                    d.move_layers_to_set(&[id], set)?;
                }
                d.set_fill_map(id, channel, Some(source))?;
                if let Some((parent, position)) = at {
                    move_into(d, id, parent, position)?;
                }
                Ok(id)
            });
        match created {
            Ok(id) => {
                self.session.mark_modified();
                self.select_new(id);
                if let Some((Some(parent), _)) = at {
                    self.m2.collapsed.remove(&parent);
                }
                self.info(
                    Source::Assets,
                    format!("{}: {name}", lang.t("shelf-placed")),
                );
            }
            Err(e) => self.cannot(
                crate::notice::Kind::of_core(&e),
                Source::Assets,
                super::Attempt::Place,
                Some(name),
                &lang.core_error(&e),
            ),
        }
    }
}

/// 足したレイヤーを、グループ `parent`（None は一番上の段）の兄弟の中の位置へ（None は一番上）。
fn move_into(
    d: &mut Document,
    id: LayerId,
    parent: Option<LayerId>,
    position: Option<usize>,
) -> Result<(), CoreError> {
    let set = d.layer(id).ok_or(CoreError::LayerNotFound)?.layer_set();
    let others = d
        .children_in_set(set, parent)?
        .into_iter()
        .filter(|c| *c != id)
        .count();
    d.move_layer_to(id, parent, position.map_or(others, |p| p.min(others)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::mesh_maps::MeshMapProvenance;

    /// 値と覆いが画素ごとに違う、`w`×`h` の焼いたマップ（覆いの無い画素が混ざる）。
    fn map(kind: MeshMapKind, w: i32, h: i32) -> BakedMeshMap {
        let provenance = MeshMapProvenance {
            kind,
            engine_version: 1,
            mesh_hash: String::new(),
            topology_hash: String::new(),
            uv_channel: 0,
            width: w,
            height: h,
            target_slot: 0,
            target_slots: vec![0],
            padding: 0,
            antialiasing: 1,
            settings_key: String::new(),
            space: String::new(),
            pose: String::new(),
            source: String::new(),
            bounds_min: [0.0; 3],
            bounds_max: [1.0; 3],
        };
        let texels = (w * h) as usize;
        let data = (0..texels * kind.channels())
            .map(|i| (i as u32).wrapping_mul(2654435761u32).wrapping_shr(16) as u16)
            .collect();
        let coverage = (0..texels).map(|i| u8::from(i % 7 != 3)).collect();
        BakedMeshMap::new(provenance, data, coverage).unwrap()
    }

    fn thumb(map: &BakedMeshMap) -> Picture {
        let source = Texels {
            data: map.data(),
            coverage: map.coverage(),
            channels: map.channels(),
            width: map.width(),
            height: map.height(),
        };
        thumb_picture(&source, || false).unwrap()
    }

    /// 今までの作り（元の大きさの RGBA8 を作ってから縮める）。
    fn through_full_size(map: &BakedMeshMap) -> Picture {
        let img = image::RgbaImage::from_raw(
            map.width() as u32,
            map.height() as u32,
            map.to_rgba8(false),
        )
        .unwrap();
        super::super::image_picture(&img)
    }

    #[test]
    fn a_small_map_gives_the_same_pixels_as_going_through_the_full_size_buffer() {
        for kind in [
            MeshMapKind::Curvature,
            MeshMapKind::WorldNormal,
            MeshMapKind::Position,
        ] {
            for (w, h) in [(16, 16), (5, 3), (64, 64), (40, 64), (1, 1), (64, 17)] {
                let m = map(kind, w, h);
                let (a, b) = (thumb(&m), through_full_size(&m));
                assert_eq!((a.width, a.height), (b.width, b.height), "{kind:?} {w}x{h}");
                assert_eq!(a.rgba, b.rgba, "{kind:?} {w}x{h}");
            }
        }
    }

    #[test]
    fn a_large_map_is_averaged_close_to_the_old_filter_and_keeps_transparent_pixels() {
        let m = map(MeshMapKind::WorldNormal, 300, 200);
        let (a, b) = (thumb(&m), through_full_size(&m));
        assert_eq!((a.width, a.height), (b.width, b.height));
        assert_eq!((a.width, a.height), (64, 43));
        // 覆いが 6/7 の画素なので、平均の透明度はどの画素も満ちている物と空の物の間に入る
        let mean = |p: &Picture| {
            p.rgba.chunks(4).map(|c| c[3] as f64).sum::<f64>() / (p.width * p.height) as f64
        };
        assert!(
            (mean(&a) - mean(&b)).abs() < 3.0,
            "{} {}",
            mean(&a),
            mean(&b)
        );
        let worst = a
            .rgba
            .iter()
            .zip(&b.rgba)
            .map(|(x, y)| x.abs_diff(*y))
            .max()
            .unwrap();
        assert!(worst < 90, "{worst}");
    }

    #[test]
    fn the_scratch_for_the_largest_bake_is_the_size_of_the_thumbnail() {
        // 焼ける最大（8192²）でも、一時に持つ物はサムネイルと 1 行ぶんの合計だけ（元の大きさの RGBA8 は 256 MiB）
        let plan = Plan::new(8192, 8192);
        assert_eq!((plan.xs.len(), plan.ys.len()), (64, 64));
        assert_eq!(plan.scratch_bytes(), 64 * 64 * 4 + 64 * 16);
        assert!(plan.scratch_bytes() < 32 * 1024);
        assert!(plan
            .xs
            .iter()
            .chain(&plan.ys)
            .all(|&(a, b)| a < b && b <= 8192));
        // 横長・細長でも 1 以上で、範囲が元の外へ出ない
        let thin = Plan::new(8192, 3);
        assert_eq!((thin.xs.len(), thin.ys.len()), (64, 1));
        assert!(thin.ys.iter().all(|&(a, b)| a < b && b <= 3));
    }

    #[test]
    fn a_stop_is_seen_at_every_row_and_gives_no_thumbnail() {
        let m = map(MeshMapKind::Curvature, 512, 512);
        let source = Texels {
            data: m.data(),
            coverage: m.coverage(),
            channels: m.channels(),
            width: m.width(),
            height: m.height(),
        };
        assert!(thumb_picture(&source, || true).is_none());
        let seen = std::cell::Cell::new(0);
        let picture = thumb_picture(&source, || {
            seen.set(seen.get() + 1);
            seen.get() > 10
        });
        assert!(picture.is_none());
        assert_eq!(seen.get(), 11);
        let info = inspect(Some(&m), || true);
        assert!(matches!(info.thumb, Thumb::None));
        assert_eq!((info.width, info.height), (512, 512));
    }
}
