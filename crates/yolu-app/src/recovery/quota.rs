//! 復旧が使うディスクの量の数え方と、上限を超えたときの消し方。
//!
//! **数え方**: 置き場の根の下の、プールの名前のフォルダ（`pool::pools`）ごとに、ファイルの長さの合計（共有の中身 `contents/` も、
//! 作りかけも、印のファイルも含む。シンボリックリンクは辿らない）。根の下のほかのもの（Unity 版の `recovery-<guid>` を含む）は
//! 数えず、消しもしない。別のウィンドウが動かしているプールは数えるが、消さない。
//!
//! **消し方**（`enforce`）: 合計が上限（`Limits::cap`）を超えているときだけ、古い世代から（世代の名前の順 = 時刻の順。プールをまたぐ）
//! 消す。消すのは「消すと空く量」が 0 でない世代で、世代が使う共有の中身は、残る世代のどれも使わなくなったものだけが空く
//! （実際の削除は `GenerationStore::remove_generation`。使われなくなった中身をそこで消す）。守る世代:
//! - この実行のプールの、いちばん新しい読める世代
//! - 落ちた実行のプールごとの、いちばん新しい読める世代（その世代が 30 日より古くなるまで。それ以降はほかの世代と同じ扱いで、
//!   上限を超えているときだけ古いほうから消える）
//!
//! 世代を消す前に、動いていないプール（`.save.lock` が取れるもの）の、落ちた書き込みの残り（`.staging-*`・どの世代も使わない
//! 共有の中身。`GenerationStore::reclaim_abandoned`）を片付ける。世代を消しても減らず、情報も失わないので、先にこれで収まるなら
//! 世代は 1 つも消さない。読めない（壊れた）世代は、残ると共有の中身が消せなくなるので、読める世代より先に消す。
//!
//! 守る世代だけで上限を超えているときは、それ以上消さず、超えたままにする（`Trimmed::over`）。消した回のあとに実際の量が見積もりより
//! 減っていなければ（見積もりが外れている）、それ以上は世代を消さない。
//! 残す数（`generations_to_keep`）による整理は別で、この上限は、数の整理を通ったあとに足す 2 つ目の歯止め。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yolu_io::{Footprint, GenerationFootprint, GenerationStore};

use super::pool::{self, Kind};
use super::settings::DiskBudget;
use super::space::{DiskSpace, SpaceProbe};

/// 落ちた実行の最新の世代を、利用者が捨てなくても守る日数（設定にしない定数）。
pub const CRASHED_KEEP_DAYS: u64 = 30;
const DAY_MS: u64 = 86_400_000;

/// 上限を決めるもの（利用者の選び・空きの確かめ）。
#[derive(Clone)]
pub struct Limits {
    pub budget: DiskBudget,
    pub probe: SpaceProbe,
    /// 試験用: 上限のバイト数を直に指定する（選びと空きを見ない）。
    pub cap_override: Option<u64>,
}

impl Limits {
    /// 置き場のあるボリュームの空き。
    pub fn space(&self, root: &Path) -> Option<DiskSpace> {
        (self.probe)(root)
    }

    /// 上限（バイト）。`used` は復旧が今使っている量（自動は、空き + 使っている量の 10% を見る）。
    pub fn cap(&self, root: &Path, used: u64) -> u64 {
        self.cap_override
            .unwrap_or_else(|| self.budget.cap(self.space(root).map(|d| d.available), used))
    }
}

/// 復旧が使っている量（バイト）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Usage {
    /// この実行のプール。
    pub own: u64,
    /// 落ちた実行のプール。
    pub crashed: u64,
    /// 正しく閉じた実行のプール。
    pub closed: u64,
    /// 別のウィンドウが動かしているプール（数えるだけで消さない）。
    pub others: u64,
}

impl Usage {
    pub fn total(&self) -> u64 {
        self.own + self.crashed + self.closed + self.others
    }
}

/// 根の下のプールが使っている量を種類ごとに数える。`own` はこの実行のプール。
pub fn usage(root: &Path, own: Option<&Path>) -> Usage {
    let mut usage = Usage::default();
    for pool in pool::pools(root) {
        let bytes = GenerationStore::new(&pool).disk_bytes();
        if Some(pool.as_path()) == own {
            usage.own += bytes;
            continue;
        }
        match pool::kind(&pool) {
            Kind::Live => usage.others += bytes,
            Kind::Crashed => usage.crashed += bytes,
            Kind::Closed => usage.closed += bytes,
        }
    }
    usage
}

/// 整理の結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Trimmed {
    /// 消した世代の数。
    pub generations: usize,
    /// 整理のあとも上限を超えているか（守る世代だけで超えている・別の窓が使っている・消せなかった）。
    pub over: bool,
}

struct PoolView {
    path: PathBuf,
    kind: Kind,
    is_own: bool,
    footprint: Footprint,
}

struct Plan {
    deletions: Vec<(PathBuf, String)>,
    over: bool,
    /// 計画の時点で実際に使っている量（残りの片付けのあと）。
    used: u64,
    /// 計画どおりに消したあとの、見積もった量。
    expected: u64,
}

/// 守る世代の名前。
fn protected(pool: &PoolView, now_ms: u64) -> Option<&str> {
    let newest = pool.footprint.generations.iter().find(|g| g.problem.is_none())?;
    let keep = if pool.is_own {
        true
    } else if pool.kind == Kind::Crashed {
        // 時刻が読めない名前は、古いと言えないので守る
        newest
            .time_ms
            .is_none_or(|t| now_ms.saturating_sub(t) < CRASHED_KEEP_DAYS * DAY_MS)
    } else {
        false
    };
    keep.then_some(newest.id.as_str())
}

/// 世代を消すと空く量（自分だけが使う共有の中身と、世代のフォルダの中）。
fn freed(pool: usize, g: &GenerationFootprint, refs: &HashMap<(usize, &str), usize>) -> (u64, u64) {
    let exclusive: u64 = g
        .contents
        .iter()
        .filter(|(hash, _)| refs.get(&(pool, hash.as_str())).copied() == Some(1))
        .map(|(_, len)| len)
        .sum();
    (exclusive, exclusive + g.own_bytes)
}

fn plan(root: &Path, limits: &Limits, own: Option<&Path>, now_ms: u64) -> Plan {
    // まず量だけを数える（書き置きのたびに呼ばれる。収まっているときは、世代の中身を調べない）
    let mut found: Vec<(PathBuf, Kind, bool)> = Vec::new();
    let mut used = 0u64;
    for path in pool::pools(root) {
        let is_own = own == Some(path.as_path());
        // 自分のプールは、自分のロックで「使用中」になるので種類を聞かない
        let kind = if is_own { Kind::Closed } else { pool::kind(&path) };
        used += GenerationStore::new(&path).disk_bytes();
        found.push((path, kind, is_own));
    }
    let mut cap = limits.cap(root, used);
    if used <= cap {
        return Plan { deletions: Vec::new(), over: false, used, expected: used };
    }
    // 世代を消す前に、落ちた書き込みの残りを片付ける（ロックを持たれているプール・動いているプールは飛ばす）。
    // 世代を消しても減らないので、見積もりが外れる原因になる
    let mut reclaimed = 0u64;
    for (path, kind, _) in &found {
        if *kind != Kind::Live {
            reclaimed += GenerationStore::new(path).reclaim_abandoned().unwrap_or(0);
        }
    }
    if reclaimed > 0 {
        used = found.iter().map(|(path, _, _)| GenerationStore::new(path).disk_bytes()).sum();
        cap = limits.cap(root, used);
        if used <= cap {
            return Plan { deletions: Vec::new(), over: false, used, expected: used };
        }
    }
    let mut pools: Vec<PoolView> = Vec::new();
    for (path, kind, is_own) in found {
        let Ok(footprint) = GenerationStore::new(&path).footprint() else {
            continue;
        };
        pools.push(PoolView { path, kind, is_own, footprint });
    }
    // 共有の中身を、いくつの世代が使っているか（プールごと。共有は同じプールの中だけ）
    let mut refs: HashMap<(usize, &str), usize> = HashMap::new();
    for (i, pool) in pools.iter().enumerate() {
        for g in &pool.footprint.generations {
            for (hash, _) in &g.contents {
                *refs.entry((i, hash.as_str())).or_default() += 1;
            }
        }
    }
    // 消してよい世代を、古い順に
    let mut candidates: Vec<(usize, &GenerationFootprint)> = Vec::new();
    for (i, pool) in pools.iter().enumerate() {
        if pool.kind == Kind::Live {
            continue;
        }
        let keep = protected(pool, now_ms);
        candidates.extend(
            pool.footprint
                .generations
                .iter()
                .filter(|g| Some(g.id.as_str()) != keep)
                .map(|g| (i, g)),
        );
    }
    // 壊れた世代を先に（残すと、どの世代の中身も消せない）。そのあとは古い順
    candidates.sort_by(|a, b| {
        (a.1.problem.is_none(), &a.1.id, &pools[a.0].path).cmp(&(b.1.problem.is_none(), &b.1.id, &pools[b.0].path))
    });
    let mut chosen = vec![false; candidates.len()];
    let mut remaining = used;
    // 1 つ選ぶ: 使っている中身の数を減らし、空く量を引く
    fn pick<'a>(
        k: usize,
        pool: usize,
        g: &'a GenerationFootprint,
        freed: u64,
        chosen: &mut [bool],
        remaining: &mut u64,
        refs: &mut HashMap<(usize, &'a str), usize>,
    ) {
        chosen[k] = true;
        for (hash, _) in &g.contents {
            if let Some(n) = refs.get_mut(&(pool, hash.as_str())) {
                *n = n.saturating_sub(1);
            }
        }
        *remaining = remaining.saturating_sub(freed);
    }
    'fit: while remaining > cap {
        // 古い順に、消すと空く世代を消す（全部がほかの世代と共有の世代は、消しても空かないので飛ばす。壊れた世代は空く量が
        // 0 でも消す: 残ると、どの世代の中身も消せなくなる）
        let mut progressed = false;
        for k in 0..candidates.len() {
            if chosen[k] {
                continue;
            }
            let (pi, g) = candidates[k];
            let shared = !g.contents.is_empty();
            let (exclusive, total) = freed(pi, g, &refs);
            if g.problem.is_none() && ((shared && exclusive == 0) || total == 0) {
                continue;
            }
            pick(k, pi, g, total, &mut chosen, &mut remaining, &mut refs);
            progressed = true;
            if remaining <= cap {
                break 'fit;
            }
        }
        if progressed {
            continue;
        }
        // 空く世代が無いまま超えている（同じ中身の世代どうしが全部を持っている）: いちばん古い 1 つを消して、相方を空く世代にする
        match (0..candidates.len()).find(|k| !chosen[*k]) {
            Some(k) => {
                let (pi, g) = candidates[k];
                let (_, total) = freed(pi, g, &refs);
                pick(k, pi, g, total, &mut chosen, &mut remaining, &mut refs);
            }
            None => break,
        }
    }
    let deletions = candidates
        .iter()
        .zip(&chosen)
        .filter(|(_, c)| **c)
        .map(|((pi, g), _)| (pools[*pi].path.clone(), g.id.clone()))
        .collect();
    Plan { deletions, over: remaining > cap, used, expected: remaining }
}

/// 合計が上限を超えていれば、まず落ちた書き込みの残りを片付け、それでも超えていれば古い世代から消して収める。世代を消せなかった
/// （別の書き手が置き場を使っている・権限）ものは飛ばし、次の機会にやり直す。消したあとに、実際の量を数え直して、収まっていなければ
/// もう 1 度（消せたものがあるあいだ。ただし前の回で実際の量が見積もりより減っていなければ、見積もりが外れているので消さない）。
pub fn enforce(root: &Path, limits: &Limits, own: Option<&Path>, now_ms: u64) -> Trimmed {
    let mut report = Trimmed::default();
    // 前の回の、計画どおりに消したあとの見積もり
    let mut expected: Option<u64> = None;
    loop {
        let plan = plan(root, limits, own, now_ms);
        report.over = plan.over;
        if plan.deletions.is_empty() {
            break;
        }
        if expected.is_some_and(|e| plan.used > e) {
            // 世代を消したのに、見積もったほど量が減っていない（消しても空かないものが残っている）。これ以上は消さない
            report.over = true;
            break;
        }
        expected = Some(plan.expected);
        let mut removed = 0;
        let mut touched: Vec<PathBuf> = Vec::new();
        for (pool, id) in &plan.deletions {
            if GenerationStore::new(pool).remove_generation(id).is_ok() {
                removed += 1;
                if !touched.contains(pool) {
                    touched.push(pool.clone());
                }
            }
        }
        // 世代が無くなったプールは（この実行のものを除いて）フォルダごと消す
        for pool in touched {
            if Some(pool.as_path()) != own && GenerationStore::new(&pool).list().is_ok_and(|g| g.is_empty()) {
                let _ = std::fs::remove_dir_all(&pool);
            }
        }
        report.generations += removed;
        if removed == 0 {
            // 消せなかった。収まっていない印のまま終える
            report.over = true;
            break;
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Arc;
    use yolu_io::{utc_stamp, CommitOptions, Files, StoreError};

    struct Root(PathBuf);
    impl Root {
        fn new(tag: &str) -> Root {
            use std::sync::atomic::{AtomicU32, Ordering};
            static N: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "yolu-quota-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&dir).unwrap();
            Root(dir)
        }
    }
    impl Drop for Root {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    const SIZE: usize = 10_000;

    fn limits(cap: u64) -> Limits {
        Limits {
            budget: DiskBudget::Auto,
            probe: Arc::new(|_| None),
            cap_override: Some(cap),
        }
    }

    /// 根の下にプールを作る（名前は今の時刻 + 通し番号）。
    fn new_pool(root: &Root) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let dir = root.0.join(format!("{}-{:016x}", utc_stamp(ms), n));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 世代を 1 つ足す。`unique` は世代だけの中身（SIZE バイト）、`shared` はほかの世代と同じ中身（`size` バイト）。
    fn add(pool: &Path, unique: u8, shared: Option<(u8, usize)>) -> String {
        let store = GenerationStore::new(pool);
        let token = store.token().ok();
        let mut files = Files::new();
        files.insert("document.utpaint".into(), Arc::from(vec![unique; SIZE]));
        if let Some((value, size)) = shared {
            files.insert("shared.bin".into(), Arc::from(vec![value; size]));
        }
        store
            .commit(
                &files,
                &CommitOptions { expected: token.as_deref(), keep: None, share: true },
            )
            .unwrap()
            .id
    }
    fn ids(pool: &Path) -> Vec<String> {
        let mut v: Vec<String> = GenerationStore::new(pool).list().unwrap().into_iter().map(|g| g.id).collect();
        v.sort();
        v
    }
    fn mark_crashed(pool: &Path) {
        fs::write(pool.join(pool::CRASHED), "state=dirty\n").unwrap();
    }
    fn now() -> u64 {
        pool::now_ms()
    }
    fn used(root: &Root) -> u64 {
        usage(&root.0, None).total()
    }

    #[test]
    fn nothing_is_removed_while_the_total_fits_under_the_cap() {
        let root = Root::new("fits");
        let a = new_pool(&root);
        add(&a, 1, None);
        add(&a, 2, None);
        let before = used(&root);
        let report = enforce(&root.0, &limits(before), None, now());
        assert_eq!(report, Trimmed::default());
        assert_eq!(ids(&a).len(), 2);
    }

    #[test]
    fn the_oldest_generations_go_first_across_pools_and_the_newest_of_this_run_and_of_each_crash_stay() {
        let root = Root::new("order");
        let (p1, p2, own, crashed) = (new_pool(&root), new_pool(&root), new_pool(&root), new_pool(&root));
        let g1 = add(&p1, 1, None);
        let g2 = add(&p1, 2, None);
        let g3 = add(&p2, 3, None);
        let g4 = add(&own, 4, None);
        let g5 = add(&own, 5, None);
        let g6 = add(&crashed, 6, None);
        let g7 = add(&crashed, 7, None);
        mark_crashed(&crashed);
        let total = used(&root);
        // 4 世代ぶんを消せば収まる上限（古い順に g1・g2・g3・g4）
        let cap = total - 3 * SIZE as u64 - 1000;
        let report = enforce(&root.0, &limits(cap), Some(&own), now());
        assert_eq!(report, Trimmed { generations: 4, over: false });
        assert!(!p1.exists() && !p2.exists(), "世代が無くなったプールは消える");
        assert_eq!(ids(&own), vec![g5.clone()], "この実行の最新は残る");
        assert_eq!(ids(&crashed), vec![g6.clone(), g7.clone()], "上限に収まったので、落ちた実行の古い世代はまだある");
        let _ = (g1, g2, g3, g4);
        assert!(used(&root) <= cap);
        // 消したあとの置き場は読める（ポインタは残った世代を指す）
        assert_eq!(GenerationStore::new(&own).load().unwrap().id, g5);
    }

    #[test]
    fn a_cap_below_what_the_kept_generations_use_removes_everything_else_and_says_it_is_still_over() {
        let root = Root::new("over");
        let (closed, own, crashed) = (new_pool(&root), new_pool(&root), new_pool(&root));
        add(&closed, 1, None);
        add(&closed, 2, None);
        add(&own, 3, None);
        let newest_own = add(&own, 4, None);
        add(&crashed, 5, None);
        let newest_crashed = add(&crashed, 6, None);
        mark_crashed(&crashed);
        let report = enforce(&root.0, &limits(1), Some(&own), now());
        assert!(report.over, "守る世代だけで上限を超えている");
        assert_eq!(report.generations, 4);
        assert!(!closed.exists());
        assert_eq!(ids(&own), vec![newest_own]);
        assert_eq!(ids(&crashed), vec![newest_crashed]);
        // 2 回目は何も消さない（守る世代しか無い）
        assert_eq!(enforce(&root.0, &limits(1), Some(&own), now()), Trimmed { generations: 0, over: true });
    }

    #[test]
    fn a_shared_content_is_removed_only_when_no_generation_uses_it_any_more() {
        let root = Root::new("shared");
        let pool = new_pool(&root);
        const BIG: usize = 50_000;
        let g1 = add(&pool, 1, Some((9, BIG)));
        let g2 = add(&pool, 2, Some((9, BIG)));
        let g3 = add(&pool, 3, Some((9, BIG)));
        let contents = |pool: &Path| fs::read_dir(pool.join("contents")).map(|d| d.count()).unwrap_or(0);
        assert_eq!(contents(&pool), 4, "正本が 3 つと、共有の中身が 1 つ");
        // g1 を消せば収まる上限: 世代の正本だけが空き、共有の中身は残る
        let total = used(&root);
        let report = enforce(&root.0, &limits(total - SIZE as u64 / 2), None, now());
        assert_eq!(report, Trimmed { generations: 1, over: false });
        assert_eq!(ids(&pool), vec![g2.clone(), g3.clone()]);
        assert_eq!(contents(&pool), 3);
        assert!(GenerationStore::new(&pool).load_generation(&g3).is_ok(), "残る世代は共有の中身ごと読める");
        let _ = g1;
        // 全部を消す上限: 最後の世代が消えたあとで、共有の中身も消える（プールごと）
        let report = enforce(&root.0, &limits(1), None, now());
        assert_eq!(report.generations, 2);
        assert!(!pool.exists());
        assert!(matches!(GenerationStore::new(&pool).load(), Err(StoreError::NoGeneration) | Err(StoreError::Io(_))));
    }

    #[test]
    fn a_generation_that_shares_everything_does_not_free_anything_and_is_kept_while_others_can_be() {
        let root = Root::new("skip");
        let pool = new_pool(&root);
        const BIG: usize = 40_000;
        // g2 の正本は g3 と同じ（変えて戻した）。g1 だけが自分の正本を持つ
        let g1 = add(&pool, 1, Some((9, BIG)));
        let g2 = add(&pool, 2, Some((9, BIG)));
        let g3 = add(&pool, 2, Some((9, BIG)));
        let own = new_pool(&root);
        add(&own, 7, None);
        // 世代 1 つぶんだけ超えている。古い順は g1 → g2。g1 が空く
        let total = used(&root);
        let report = enforce(&root.0, &limits(total - SIZE as u64 / 2), Some(&own), now());
        assert_eq!(report, Trimmed { generations: 1, over: false });
        assert_eq!(ids(&pool), vec![g2.clone(), g3.clone()], "g2 を消しても空かないので、空く g1 を消した");
        // さらに削る: g2 と g3 は同じ正本を持つので、どちらを消しても空かない。いちばん古い g2 を消して、g3 の正本を空く側にする
        let report = enforce(&root.0, &limits(1), Some(&own), now());
        assert_eq!(report.generations, 2, "{report:?}");
        assert!(!pool.exists());
        let _ = g1;
    }

    #[test]
    fn a_crashed_runs_newest_is_kept_for_thirty_days_and_after_that_only_the_cap_decides() {
        let root = Root::new("days");
        let crashed = new_pool(&root);
        let older = add(&crashed, 1, None);
        let newest = add(&crashed, 2, None);
        mark_crashed(&crashed);
        let t = yolu_io::generation_time_ms(&newest).unwrap();
        let day = 86_400_000u64;
        // 29 日: 上限がどれだけ小さくても最新は残る。古い世代は消える
        let report = enforce(&root.0, &limits(1), None, t + 29 * day);
        assert_eq!(report, Trimmed { generations: 1, over: true });
        assert_eq!(ids(&crashed), vec![newest.clone()]);
        // 31 日を過ぎても、上限に収まっているなら消さない
        let report = enforce(&root.0, &limits(u64::MAX), None, t + 31 * day);
        assert_eq!(report, Trimmed::default());
        assert_eq!(ids(&crashed), vec![newest.clone()]);
        // 31 日を過ぎて上限を超えていれば、ほかの世代と同じに古いほうから消える
        let report = enforce(&root.0, &limits(1), None, t + 31 * day);
        assert_eq!(report, Trimmed { generations: 1, over: false });
        assert!(!crashed.exists());
        let _ = older;
    }

    #[test]
    fn a_pool_another_window_is_running_counts_but_is_never_touched_and_unrelated_folders_are_left_alone() {
        let root = Root::new("live");
        let live = new_pool(&root);
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(live.join(pool::LOCK))
            .unwrap();
        lock.try_lock().unwrap();
        let live_g = add(&live, 1, None);
        let closed = new_pool(&root);
        add(&closed, 2, None);
        // プールの名前ではないフォルダ（Unity 版の復旧の置き場の名前・利用者のもの）は、数えず、消さない
        let foreign = root.0.join("recovery-0123456789abcdef0123456789abcdef");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("big.bin"), vec![0u8; 100_000]).unwrap();
        let usage = usage(&root.0, None);
        assert!(usage.others >= SIZE as u64 && usage.closed >= SIZE as u64);
        assert_eq!(usage.own + usage.crashed, 0);
        assert!(usage.total() < 100_000, "プールの外は数えない");
        let report = enforce(&root.0, &limits(1), None, now());
        assert_eq!(report, Trimmed { generations: 1, over: true }, "動いているプールは消せないので、超えたまま");
        assert_eq!(ids(&live), vec![live_g]);
        assert!(!closed.exists());
        assert!(foreign.join("big.bin").is_file());
        drop(lock);
    }

    #[test]
    fn a_generation_that_cannot_be_removed_now_is_skipped_and_the_rest_still_goes_without_looping() {
        let root = Root::new("busy");
        let (busy, free) = (new_pool(&root), new_pool(&root));
        let kept = add(&busy, 1, None);
        add(&free, 2, None);
        // 別の書き手が置き場のロックを持っている（書いている最中）: その世代は消せない
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(busy.join(".save.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        let report = enforce(&root.0, &limits(1), None, now());
        assert_eq!(report, Trimmed { generations: 1, over: true }, "消せたものだけ消し、収まっていないと知らせて終わる");
        assert_eq!(ids(&busy), vec![kept.clone()]);
        assert!(!free.exists());
        drop(lock);
        // ロックが外れれば、次の機会に消える
        assert_eq!(enforce(&root.0, &limits(1), None, now()), Trimmed { generations: 1, over: false });
        assert!(!busy.exists());
    }

    #[test]
    fn a_damaged_generation_is_an_ordinary_candidate_and_never_the_one_that_is_kept() {
        let root = Root::new("damaged");
        let pool = new_pool(&root);
        let a = add(&pool, 1, None);
        let b = add(&pool, 2, None);
        let c = add(&pool, 3, None);
        // いちばん新しい世代が壊れている: 守るのは、その前の読める世代
        fs::remove_file(pool.join("generations").join(&c).join("manifest.sha256")).unwrap();
        let own = pool.clone();
        let report = enforce(&root.0, &limits(1), Some(&own), now());
        assert!(report.over);
        assert_eq!(ids(&pool), vec![b.clone()], "読める最新（b）が残り、壊れた c と古い a は消える");
        let _ = a;
    }

    /// 落ちた書き込みの残り: manifest の無い作りかけ（`<札>.pending` 入り）。
    fn leave_staging(pool: &Path, tag: &str, bytes: usize) -> PathBuf {
        let staging = pool.join(format!(".staging-20200101T000000000-{tag}"));
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("y.pending"), vec![9u8; bytes]).unwrap();
        staging
    }

    #[test]
    fn the_leftovers_of_a_crashed_write_are_reclaimed_before_any_generation_goes() {
        let root = Root::new("leftover");
        let (pool, crashed) = (new_pool(&root), new_pool(&root));
        let ids_before = {
            add(&pool, 1, None);
            add(&pool, 2, None);
            add(&pool, 3, None);
            add(&crashed, 4, None);
            mark_crashed(&crashed);
            (ids(&pool), ids(&crashed))
        };
        // 落ちた書き込みの残り（大きい）と、確定の前に落ちて誰も使わない中身
        let stale = leave_staging(&pool, "a", 3 * SIZE);
        let stale_crashed = leave_staging(&crashed, "b", SIZE);
        let orphan = pool.join("contents").join(format!("{}.bin", "ab".repeat(32)));
        fs::write(&orphan, vec![7u8; SIZE / 2]).unwrap();
        let total = used(&root);
        // 残りを片付ければ収まる上限。世代を 1 つでも消せば、必要以上に消している
        let cap = total - 3 * SIZE as u64 - SIZE as u64 / 2;
        let report = enforce(&root.0, &limits(cap), None, now());
        assert_eq!(report, Trimmed::default(), "世代は消さず、収まった");
        assert_eq!((ids(&pool), ids(&crashed)), ids_before);
        assert!(!stale.exists() && !stale_crashed.exists() && !orphan.exists());
        assert!(used(&root) <= cap, "実際の量が上限へ向かって減る");
        // 残りの無い置き場は、収まっているあいだ何も起きない
        assert_eq!(enforce(&root.0, &limits(cap), None, now()), Trimmed::default());
    }

    #[test]
    fn after_reclaiming_only_what_is_still_over_is_taken_from_the_oldest_generations() {
        let root = Root::new("leftover-then-cap");
        let pool = new_pool(&root);
        let (g1, g2, g3) = (add(&pool, 1, None), add(&pool, 2, None), add(&pool, 3, None));
        let stale = leave_staging(&pool, "c", 5 * SIZE);
        let total = used(&root);
        // 残り（5 SIZE）を片付けて、さらに世代 1 つぶん
        let cap = total - 5 * SIZE as u64 - SIZE as u64 / 2;
        let report = enforce(&root.0, &limits(cap), None, now());
        assert_eq!(report, Trimmed { generations: 1, over: false });
        assert_eq!(ids(&pool), vec![g2, g3], "いちばん古い 1 つだけ");
        assert!(!stale.exists());
        assert!(used(&root) <= cap);
        let _ = g1;
    }

    #[test]
    fn leftovers_in_a_pool_another_window_runs_or_one_that_is_being_written_are_not_touched() {
        let root = Root::new("leftover-live");
        let (live, busy, closed) = (new_pool(&root), new_pool(&root), new_pool(&root));
        add(&live, 1, None);
        add(&busy, 2, None);
        add(&closed, 3, None);
        let live_lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(live.join(pool::LOCK))
            .unwrap();
        live_lock.try_lock().unwrap();
        let save_lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(busy.join(".save.lock"))
            .unwrap();
        save_lock.try_lock().unwrap();
        let stale_live = leave_staging(&live, "d", SIZE);
        let stale_busy = leave_staging(&busy, "e", SIZE);
        let stale_closed = leave_staging(&closed, "f", SIZE);
        let report = enforce(&root.0, &limits(1), None, now());
        assert!(report.over);
        assert!(stale_live.exists(), "動いているプールは触らない");
        assert!(stale_busy.exists(), "書いている最中かもしれないので触らない");
        assert!(!stale_closed.exists() && !closed.exists());
        drop((live_lock, save_lock));
    }

    #[test]
    fn a_round_that_frees_less_than_estimated_stops_the_removal_instead_of_emptying_the_pool() {
        let root = Root::new("estimate");
        let pool = new_pool(&root);
        let (g1, g2, g3) = (add(&pool, 1, None), add(&pool, 2, None), add(&pool, 3, None));
        // 共有の中身を消せなくするもの（`.staging-` の名前の、フォルダではないもの。整理は読めない作りかけとして、何も消さない）
        fs::write(pool.join(".staging-blocker"), b"x").unwrap();
        let total = used(&root);
        // g1 を消せば収まる見積もり。実際は g1 の正本が空かない
        let cap = total - SIZE as u64 / 2;
        let report = enforce(&root.0, &limits(cap), Some(&pool), now());
        assert_eq!(report, Trimmed { generations: 1, over: true }, "{report:?}");
        assert_eq!(ids(&pool), vec![g2, g3], "見積もりが外れたら、残りの世代を消し続けない");
        let _ = g1;
    }

    #[test]
    fn a_damaged_generation_goes_before_the_readable_ones_so_the_contents_it_blocked_can_be_freed() {
        let root = Root::new("damaged-first");
        let pool = new_pool(&root);
        let g1 = add(&pool, 1, None);
        let g2 = add(&pool, 2, None);
        let g3 = add(&pool, 3, None);
        let g4 = add(&pool, 4, None);
        // いちばん新しい世代の manifest が無い: 残ると、どの世代の中身も消せない
        fs::remove_file(pool.join("generations").join(&g4).join("manifest.sha256")).unwrap();
        let total = used(&root);
        // 読める世代の 1 つぶんだけ超えている
        let cap = total - SIZE as u64 / 2;
        let report = enforce(&root.0, &limits(cap), Some(&pool), now());
        assert_eq!(report, Trimmed { generations: 2, over: false }, "{report:?}");
        assert_eq!(ids(&pool), vec![g2, g3], "壊れた g4 と、いちばん古い g1");
        assert!(used(&root) <= cap, "g1 の中身も実際に空いた");
        let _ = g1;
    }

    #[test]
    fn the_usage_is_split_by_the_kind_of_pool() {
        let root = Root::new("usage");
        let (own, crashed, closed) = (new_pool(&root), new_pool(&root), new_pool(&root));
        add(&own, 1, None);
        add(&crashed, 2, None);
        mark_crashed(&crashed);
        add(&closed, 3, None);
        let u = usage(&root.0, Some(&own));
        for part in [u.own, u.crashed, u.closed] {
            assert!(part >= SIZE as u64 && part < 2 * SIZE as u64, "{u:?}");
        }
        assert_eq!(u.others, 0);
        assert_eq!(u.total(), u.own + u.crashed + u.closed);
    }
}
