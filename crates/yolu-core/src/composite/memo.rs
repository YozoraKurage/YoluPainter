//! 描いている間の「下の覚え」: ストロークの層より下の重ねた結果を、タイルごとに覚えておき、描いた層と上の層だけを重ねる。
//!
//! - 合成は層を下から 1 段ずつ重ね、段ごとに RGBA8 へ丸める。なので、描く層の段の手前までの結果（途中の値）は、そのまま RGBA8 で持てて、
//!   そこから続けた合成は、全部を下から重ねた合成とバイトで同じになる。上の層（描く層の段より後）は、描く層の結果に依る（合成モード・
//!   調整・クリッピング・通過のグループ）ので、覚えず毎回重ねる（そのタイルに画素のある上の層だけが仕事になる）。
//! - 描く層への「段の道」: 一番上の段から、描く層を含む段（グループなら、その子の段へ）をたどる。道の各段について、その段の手前までの結果を
//!   覚える。通過のグループの中身は下の結果の上へ、ほかのグループは透明の上へ重ねるので、段ごとの出発点がそれに合わせて決まる。
//!   クリッピングの組の中の層・クリッピングされたグループの中は道の終わり（その段を、組ごと毎回重ねる）。
//! - 覚えるのは、ストロークが手を付けたタイルだけ（書き出し・サムネイルのように、広い矩形の合成が通っても、描いていないタイルを覚えない）。
//!   覚えの大きさは「1 回の操作」の予算（ストロークの予算から巻き戻し用の写しを引いた残り）の中で、超えるタイルは覚えず今の道で重ねる。
//!   ストロークが進んで巻き戻しが育ち、覚えが残りに収まらなくなったら、覚えが譲って手放す（点を足すたびと、合成のたび。ブラシの予算の確かめは
//!   覚えを数えないので、覚えの都合でストロークが断られることはない）。点を足し終えた時点と合成のあとは、覚え + 巻き戻しが予算に収まる。
//!   1 点を足している最中は巻き戻しが先に育つので、その瞬間は覚えの分だけ予算を越え得る。
//! - 覚えが正しい間だけ使う: 同じストローク・チャンネル・道で、描く層の画素以外の変化の記録（`Journal` の外の印）が進んでいない間。
//!   ストロークが終わる・取り消される・別の変化があるときは捨てる。Anchor を読む効果があるときは使わない（描く層の画素が、下の層の出力を
//!   変え得るため）。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use rayon::prelude::*;

use super::{Entry, Geom, Level, Out, Plan, TileState, Worker};
use crate::types::{Channel, TileCoord};

/// 覚えの状況（試験・計測用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoStats {
    /// いま覚えているタイルの数。
    pub tiles: usize,
    /// いま覚えているバイト数。
    pub bytes: u64,
    /// 覚えから続けて合成したタイルの数（延べ）。
    pub hits: u64,
    /// 覚えを作ったタイルの数（延べ）。
    pub built: u64,
    /// 予算に収まらず、覚えず今の道で重ねたタイルの数（延べ）。
    pub skipped: u64,
    /// 巻き戻しが育って予算に収まらなくなり、覚えを手放したタイルの数（延べ）。
    pub evicted: u64,
}

#[derive(PartialEq, Eq)]
struct Key {
    stroke: u64,
    channel: Channel,
    /// 道の各段の層の番号。
    route: Vec<usize>,
    /// 描く層の画素以外の変化の記録の印。
    foreign: u64,
}

#[derive(Default)]
struct Inner {
    key: Option<Key>,
    tiles: HashMap<TileCoord, Arc<TileMemo>>,
    bytes: u64,
}

impl Inner {
    /// 覚えのバイト数が limit に収まるまで、タイルを手放す（手放した枚数を返す）。手放す順は、今回の合成に無いタイル（`keep` の外）が先、
    /// 次に今回のタイルで、どちらも座標の順（覚えを使うかどうかでバイトは変わらないので、順は結果に影響しない。試験が毎回同じ状況になる）。
    fn evict_to(&mut self, limit: u64, keep: &[TileCoord]) -> u64 {
        if self.bytes <= limit {
            return 0;
        }
        let keep: std::collections::HashSet<TileCoord> = keep.iter().copied().collect();
        let mut order: Vec<(bool, TileCoord)> =
            self.tiles.keys().map(|c| (keep.contains(c), *c)).collect();
        order.sort();
        let mut dropped = 0;
        for (_, c) in order {
            if self.bytes <= limit {
                break;
            }
            if let Some(t) = self.tiles.remove(&c) {
                self.bytes -= t.bytes();
                dropped += 1;
            }
        }
        dropped
    }
}

/// 描いている間の下の覚え（文書が持つ。中身はストロークの間だけ）。
#[derive(Default)]
pub(crate) struct Memo {
    inner: Mutex<Inner>,
    disabled: AtomicBool,
    hits: AtomicU64,
    built: AtomicU64,
    skipped: AtomicU64,
    evicted: AtomicU64,
}

impl Memo {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub(crate) fn enabled(&self) -> bool {
        !self.disabled.load(Ordering::Relaxed)
    }
    /// 覚えを使うか（既定は使う。診断・前後の比較のために切れる）。切ると覚えも捨てる。
    pub(crate) fn set_enabled(&self, on: bool) {
        self.disabled.store(!on, Ordering::Relaxed);
        if !on {
            self.clear();
        }
    }
    /// 覚えを捨てる（ストロークの終わり・取り消し）。
    pub(crate) fn clear(&self) {
        let mut g = self.lock();
        g.key = None;
        g.tiles.clear();
        g.bytes = 0;
    }
    /// 覚えを limit（ストロークの予算から巻き戻し用の写しを引いた残り）に収める。巻き戻しが育ったとき、覚えが譲る（覚えは無くても
    /// 合成の結果は同じで、ストロークの方が先に予算を使う）。ストロークの点を足すたびに、文書が呼ぶ。
    pub(crate) fn fit(&self, limit: u64) {
        let mut g = self.lock();
        let dropped = g.evict_to(limit, &[]);
        drop(g);
        if dropped > 0 {
            self.evicted.fetch_add(dropped, Ordering::Relaxed);
        }
    }
    pub(crate) fn stats(&self) -> MemoStats {
        let g = self.lock();
        MemoStats {
            tiles: g.tiles.len(),
            bytes: g.bytes,
            hits: self.hits.load(Ordering::Relaxed),
            built: self.built.load(Ordering::Relaxed),
            skipped: self.skipped.load(Ordering::Relaxed),
            evicted: self.evicted.load(Ordering::Relaxed),
        }
    }
}

/// 覚えを使う合成の依頼（文書が、描いている間だけ作る）。
pub(crate) struct MemoRequest<'m> {
    pub memo: &'m Memo,
    pub stroke: u64,
    /// 描く層の番号（`layers` の中）。
    pub layer: usize,
    pub channel: Channel,
    /// 描く層の画素以外の変化の記録の印。
    pub foreign: u64,
    /// 覚えてよい大きさ（バイト）。
    pub limit: u64,
    /// 文書の大きさ（端のタイルの大きさを決める）。
    pub width: u32,
    pub height: u32,
    /// ストロークが手を付けたタイルか（覚えるのはそのタイルだけ）。
    pub touched: Box<dyn Fn(TileCoord) -> bool + 'm>,
}

/// 1 枚のタイルの覚え: 道の各段の手前までの結果（タイルの画素の並び。行は下から上、詰めた並び）。
pub(super) struct TileMemo {
    width: usize,
    levels: Vec<Vec<u8>>,
}

impl TileMemo {
    fn bytes(&self) -> u64 {
        self.levels.iter().map(|l| l.len() as u64).sum()
    }
    /// 道の段 d の手前までの結果を、帯の行へ書く。
    fn load_level(&self, d: usize, res: &mut [u8], out: Out, g: Geom) {
        let src = &self.levels[d];
        let bytes = g.count * 4;
        for r in 0..g.rows {
            let from = ((g.y + r) * self.width + g.x) * 4;
            res[out.row(r)..out.row(r) + bytes].copy_from_slice(&src[from..from + bytes]);
        }
    }
}

/// 1 回の合成が使う覚え（タイルごと）。
pub(super) struct MemoRun {
    /// 道の各段で、描く層を含む段の位置（その段の並びの中）。
    route: Vec<usize>,
    tiles: HashMap<TileCoord, Arc<TileMemo>>,
    /// 覚えから続けるとき、重ねる仕事になる段（描く層を含む段と、その上の段。計画の段の番号ごと）。
    suffix: Vec<bool>,
}

/// 覚えから続ける印: どのタイルの覚えを、道のどの段から。
#[derive(Clone, Copy)]
pub(super) struct Resume<'m> {
    memo: &'m TileMemo,
    route: &'m [usize],
    depth: usize,
}

impl MemoRun {
    /// そのタイルが覚えから続けて重ねるなら、仕事になる段の印（そうでなければ None）。
    pub(super) fn suffix_of(&self, coord: TileCoord) -> Option<&[bool]> {
        self.tiles.contains_key(&coord).then_some(&self.suffix[..])
    }

    pub(super) fn resume(&self, coord: TileCoord) -> Option<Resume<'_>> {
        let memo = self.tiles.get(&coord)?;
        Some(Resume {
            memo,
            route: &self.route,
            depth: 0,
        })
    }
}

/// 描く層への段の道: 一番上の段から、描く層を含む段（グループは子の段へ）をたどる。各段の（位置, 層の番号）。
/// クリッピングの組の中・クリッピングされたグループの中にあるときは、その組の下地の段までで終わる。
fn find_route(entries: &[Entry], target: usize) -> Option<Vec<(usize, usize)>> {
    fn holds(e: &Entry, target: usize) -> bool {
        e.layer == target
            || e.clips.iter().any(|c| holds(c, target))
            || e.children.iter().any(|c| holds(c, target))
    }
    for (pos, e) in entries.iter().enumerate() {
        if e.layer == target || e.clips.iter().any(|c| holds(c, target)) {
            return Some(vec![(pos, e.layer)]);
        }
        if let Some(mut r) = find_route(&e.children, target) {
            r.insert(0, (pos, e.layer));
            return Some(r);
        }
    }
    None
}

/// 合成する矩形のタイルのうち、覚えてよいもの（ストロークが手を付けた・予算に収まる）の覚えを（無ければ作って）用意する。
/// 覚えが使えない（切ってある・描く層が計画に無い）ときは None（今の道で重ねる）。
pub(super) fn prepare<'a>(
    plan: &Plan<'a>,
    entries: &[Entry],
    req: &MemoRequest<'_>,
    coords: &[TileCoord],
    depth: usize,
) -> Option<MemoRun> {
    if !req.memo.enabled() {
        return None;
    }
    let route = find_route(entries, req.layer)?;
    let positions: Vec<usize> = route.iter().map(|r| r.0).collect();
    let key = Key {
        stroke: req.stroke,
        channel: req.channel,
        route: route.iter().map(|r| r.1).collect(),
        foreign: req.foreign,
    };
    let ts = plan.tile_size as u32;
    let dims = |c: TileCoord| -> (usize, usize) {
        let (x, y) = (c.x as u64 * ts as u64, c.y as u64 * ts as u64);
        (
            (req.width as u64).saturating_sub(x).min(ts as u64) as usize,
            (req.height as u64).saturating_sub(y).min(ts as u64) as usize,
        )
    };
    let mut have: HashMap<TileCoord, Arc<TileMemo>> = HashMap::new();
    let mut want: Vec<(TileCoord, (usize, usize))> = Vec::new();
    let mut skipped = 0u64;
    let evicted;
    {
        let mut g = req.memo.lock();
        if g.key.as_ref() != Some(&key) {
            g.tiles.clear();
            g.bytes = 0;
            g.key = Some(key);
        }
        // 覚えたあとにストロークの巻き戻しが育って、今の予算に収まらないなら、収まるまで手放す
        evicted = g.evict_to(req.limit, coords);
        let mut projected = g.bytes;
        for &c in coords {
            if !(req.touched)(c) {
                continue;
            }
            if let Some(t) = g.tiles.get(&c) {
                have.insert(c, t.clone());
                continue;
            }
            let (tw, th) = dims(c);
            if tw == 0 || th == 0 {
                continue;
            }
            let bytes = (positions.len() * tw * th * 4) as u64;
            if projected + bytes <= req.limit {
                projected += bytes;
                want.push((c, (tw, th)));
            } else {
                skipped += 1;
            }
        }
    }
    let hits = have.len() as u64;
    let nodes = plan.nodes.len();
    let compute = |w: &mut Worker<'a>, c: TileCoord, d: (usize, usize)| {
        plan.tile_memo(&positions, w, c, d)
    };
    let built: Vec<(TileCoord, TileMemo)> = if want.len() < 4 {
        let mut w = Worker::new(nodes, depth);
        want.iter().map(|&(c, d)| (c, compute(&mut w, c, d))).collect()
    } else {
        want.par_iter()
            .map_init(
                || Worker::new(nodes, depth),
                |w, &(c, d)| (c, compute(w, c, d)),
            )
            .collect()
    };
    let n_built = built.len() as u64;
    {
        let mut g = req.memo.lock();
        let same = g.key.as_ref().is_some_and(|k| {
            k.stroke == req.stroke && k.channel == req.channel && k.foreign == req.foreign
        });
        for (c, t) in built {
            let t = Arc::new(t);
            // 同じ文書で合成が並んで走っても、覚えの合計が予算を超えないよう、入れる時にも確かめる（入らなければ今回だけ使う）
            if same && g.bytes + t.bytes() <= req.limit && !g.tiles.contains_key(&c) {
                g.bytes += t.bytes();
                g.tiles.insert(c, t.clone());
            }
            have.insert(c, t);
        }
    }
    req.memo.hits.fetch_add(hits, Ordering::Relaxed);
    req.memo.built.fetch_add(n_built, Ordering::Relaxed);
    req.memo.skipped.fetch_add(skipped, Ordering::Relaxed);
    req.memo.evicted.fetch_add(evicted, Ordering::Relaxed);
    let suffix = plan.suffix_nodes(&positions);
    Some(MemoRun {
        route: positions,
        tiles: have,
        suffix,
    })
}

impl<'a> Plan<'a> {
    /// 覚えから続けるときに重ねる段（道の各段で、描く層を含む段から上の段。その中身も）の印を、段の番号ごとに。
    fn suffix_nodes(&self, route: &[usize]) -> Vec<bool> {
        fn mark_all(plan: &Plan<'_>, id: usize, out: &mut [bool]) {
            out[id] = true;
            for &c in plan.nodes[id].children.iter().chain(&plan.nodes[id].clips) {
                mark_all(plan, c, out);
            }
        }
        let mut out = vec![false; self.nodes.len()];
        let mut ids: &[usize] = &self.roots;
        for (d, &pos) in route.iter().enumerate() {
            if d + 1 == route.len() {
                for &id in &ids[pos..] {
                    mark_all(self, id, &mut out);
                }
                break;
            }
            // 道の途中の段（グループ）は、自分の合成の分だけ仕事。中身は次の段で数える
            out[ids[pos]] = true;
            for &id in &ids[pos + 1..] {
                mark_all(self, id, &mut out);
            }
            ids = &self.nodes[ids[pos]].children;
        }
        out
    }

    /// 1 枚のタイルの覚えを作る: 道の各段について、その段の手前までの結果（タイル全体）。段の出発点は、通過のグループの中身なら
    /// 手前の段の結果、ほかのグループの中身なら透明。
    fn tile_memo(
        &self,
        route: &[usize],
        w: &mut Worker<'a>,
        coord: TileCoord,
        (tw, th): (usize, usize),
    ) -> TileMemo {
        self.load(&self.roots, coord, &mut w.st);
        let g = Geom {
            x: 0,
            y: 0,
            rows: th,
            count: tw,
        };
        let out = Out::packed(tw * 4);
        let mut levels: Vec<Vec<u8>> = Vec::with_capacity(route.len());
        let mut ids: &[usize] = &self.roots;
        let mut start = vec![0u8; tw * th * 4];
        for (d, &pos) in route.iter().enumerate() {
            let mut buf = start;
            self.eval_rect(&ids[..pos], &mut buf, out, g, &w.st, &mut w.scratch);
            levels.push(buf.clone());
            if d + 1 == route.len() {
                break;
            }
            let n = &self.nodes[ids[pos]];
            start = if n.passes_through {
                buf
            } else {
                vec![0u8; tw * th * 4]
            };
            ids = &n.children;
        }
        TileMemo { width: tw, levels }
    }

    /// 覚えから続けて、描く層を含む段と、その上の段を重ねる（`ids` は道の段の並び）。res の中身は使わず、段の手前までの覚えで置き換える。
    #[allow(clippy::too_many_arguments)]
    pub(super) fn eval_resume(
        &self,
        ids: &[usize],
        r: Resume<'_>,
        res: &mut [u8],
        out: Out,
        g: Geom,
        st: &TileState<'a>,
        scratch: &mut [Level],
    ) {
        let pos = r.route[r.depth];
        r.memo.load_level(r.depth, res, out, g);
        let next = (r.depth + 1 < r.route.len()).then_some(Resume {
            depth: r.depth + 1,
            ..r
        });
        self.eval_node(ids[pos], res, out, g, st, scratch, next);
        self.eval_rect(&ids[pos + 1..], res, out, g, st, scratch);
    }
}
