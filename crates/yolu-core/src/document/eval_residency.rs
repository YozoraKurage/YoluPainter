//! 評価のキャッシュ（評価したブロック・Anchor のタイル・画像のミップマップ）の、予算を超えたときの捨て方。
//!
//! 入れた物は「一度だけ使われた段」に入り、入れた後にもう一度使われる（当たる・同じ鍵で作り直される）と「2 度以上使われた段」へ上がる。
//! 予算を超えたら、一度だけの段から捨て、それが尽きてから 2 度以上の段を古い順に捨てる。2 度以上の段は予算の 3/4 までで、超えた分は
//! 古い順に一度だけの段へ戻す（新しく使われ始めた物が入る余地を残す）。
//!
//! 一度だけの段の中は、捨てるたびに選び直す擬似乱数の順で捨てる（鍵と捨てた回数のハッシュ。順は鍵と回数で決まる。並列の評価では入れる順がぶれるので、当たりの数は回ごとにぶれる）。古い順に捨てると、
//! 予算より大きな範囲を同じ順で繰り返し読む使い方（全面の合成を何度もする）では、次に読む物から先に捨て続け、一度も当たらない。
//! 新しい順に捨てると、使われなくなった古い物がいつまでも残る。乱数の順なら、どちらでも一部が残って当たり、使われなくなった物も
//! 少しずつ減る。
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// 予算を超えたとき、どこまで減らすか（予算から引く割合の分母）。1 回ごとに並べ替えるので、少しだけ余分に捨てて回数を減らす。
const SLACK: u64 = 16;

pub(super) struct Residency<K, V> {
    entries: HashMap<K, Slot<V>>,
    bytes: u64,
    reused_bytes: u64,
    clock: u64,
    trims: u64,
}

struct Slot<V> {
    value: V,
    bytes: u64,
    used: u64,
    reused: bool,
}

impl<K, V> Default for Residency<K, V> {
    fn default() -> Self {
        Residency {
            entries: HashMap::new(),
            bytes: 0,
            reused_bytes: 0,
            clock: 0,
            trims: 0,
        }
    }
}

impl<K: Hash + Eq + Clone, V> Residency<K, V> {
    /// 持っているバイト数。
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// 鍵の物を、使ったことにせずに見る。
    pub fn peek(&self, key: &K) -> Option<&V> {
        self.entries.get(key).map(|s| &s.value)
    }

    /// 鍵の物が `fresh` なら、使ったことにして返す（2 度目の使用なら上の段へ）。古ければ（鍵が合わなければ）None で、何も変えない。
    pub fn hit(&mut self, key: &K, fresh: impl FnOnce(&V) -> bool) -> Option<&V> {
        self.clock += 1;
        let clock = self.clock;
        let slot = self.entries.get_mut(key)?;
        if !fresh(&slot.value) {
            return None;
        }
        slot.used = clock;
        if !slot.reused {
            slot.reused = true;
            self.reused_bytes += slot.bytes;
        }
        Some(&slot.value)
    }

    /// 入れる（同じ鍵の前の物は置き換える。置き換えは同じ所をもう一度使ったので、上の段に入れる）。予算は `trim` が見る。
    pub fn insert(&mut self, key: K, value: V, bytes: u64) {
        self.clock += 1;
        let reused = self.remove(&key).is_some();
        self.bytes += bytes;
        if reused {
            self.reused_bytes += bytes;
        }
        self.entries.insert(
            key,
            Slot {
                value,
                bytes,
                used: self.clock,
                reused,
            },
        );
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let slot = self.entries.remove(key)?;
        self.forget(&slot);
        Some(slot.value)
    }

    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        let gone: Vec<K> = self
            .entries
            .iter()
            .filter(|(k, s)| !keep(k, &s.value))
            .map(|(k, _)| k.clone())
            .collect();
        for k in gone {
            self.remove(&k);
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
        self.reused_bytes = 0;
    }

    fn forget(&mut self, slot: &Slot<V>) {
        self.bytes -= slot.bytes;
        if slot.reused {
            self.reused_bytes -= slot.bytes;
        }
    }

    /// 予算を超えていたら、予算の 15/16 まで減らす（`exact` なら予算ちょうどまで）。
    pub fn trim(&mut self, budget: u64, exact: bool) {
        if self.bytes <= budget {
            return;
        }
        let target = if exact {
            budget
        } else {
            budget - budget / SLACK
        };
        self.trims += 1;
        // 2 度以上の段が 3/4 を超えた分は、古い順に一度だけの段へ戻す
        let cap = budget / 4 * 3;
        if self.reused_bytes > cap {
            let mut reused: Vec<(u64, K)> = self
                .entries
                .iter()
                .filter(|(_, s)| s.reused)
                .map(|(k, s)| (s.used, k.clone()))
                .collect();
            reused.sort_unstable_by_key(|e| e.0);
            for (_, k) in reused {
                if self.reused_bytes <= cap {
                    break;
                }
                let slot = self.entries.get_mut(&k).expect("ある鍵");
                slot.reused = false;
                self.reused_bytes -= slot.bytes;
            }
        }
        // 一度だけの段を擬似乱数の順で、尽きたら 2 度以上の段を古い順で
        let trims = self.trims;
        let mut order: Vec<(bool, u64, K)> = self
            .entries
            .iter()
            .map(|(k, s)| {
                let rank = if s.reused { s.used } else { shuffle(k, trims) };
                (s.reused, rank, k.clone())
            })
            .collect();
        order.sort_unstable_by_key(|e| (e.0, e.1));
        for (_, _, k) in order {
            if self.bytes <= target {
                break;
            }
            self.remove(&k);
        }
    }
}

/// 鍵と捨てた回数から決まる擬似乱数（プロセスによらず同じ。`DefaultHasher::new` は固定の鍵）。
fn shuffle<K: Hash>(key: &K, trims: u64) -> u64 {
    let mut h = std::hash::DefaultHasher::new();
    key.hash(&mut h);
    trims.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `items` 個を同じ順で `passes` 回読み、回ごとの当たりの数を返す（1 個 1 バイト、予算 `budget` バイト）。
    fn scan(budget: u64, items: u64, passes: usize) -> Vec<u64> {
        let mut r: Residency<u64, ()> = Residency::default();
        (0..passes)
            .map(|_| {
                let mut hits = 0;
                for k in 0..items {
                    if r.hit(&k, |_| true).is_some() {
                        hits += 1;
                    } else {
                        r.insert(k, (), 1);
                        r.trim(budget, false);
                    }
                }
                hits
            })
            .collect()
    }

    #[test]
    fn a_scan_twice_the_budget_hits_on_the_second_pass() {
        let hits = scan(1024, 2048, 4);
        assert_eq!(hits[0], 0);
        assert!(hits[1] > 100, "2 回目に当たる: {hits:?}");
        assert!(hits[3] >= hits[1], "続けても減らない: {hits:?}");
    }

    #[test]
    fn a_scan_that_fits_always_hits() {
        assert_eq!(scan(1024, 1000, 3), vec![0, 1000, 1000]);
    }

    #[test]
    fn stays_within_the_budget_and_counts_bytes() {
        let mut r: Residency<u64, ()> = Residency::default();
        for k in 0..500 {
            r.insert(k, (), 3);
            if k % 2 == 0 {
                r.hit(&(k / 2), |_| true);
            }
            r.trim(256, false);
            assert!(r.bytes() <= 256);
            let total: u64 = r.entries.values().map(|s| s.bytes).sum();
            let reused: u64 = r
                .entries
                .values()
                .filter(|s| s.reused)
                .map(|s| s.bytes)
                .sum();
            assert_eq!((r.bytes, r.reused_bytes), (total, reused));
        }
        r.trim(0, true);
        assert_eq!(r.bytes(), 0);
        assert!(r.entries.is_empty());
    }

    #[test]
    fn something_used_again_outlives_what_was_used_once() {
        let mut r: Residency<u64, ()> = Residency::default();
        for k in 0..64 {
            r.insert(k, (), 1);
        }
        for k in 0..8 {
            r.hit(&k, |_| true);
        }
        for k in 64..1000 {
            r.insert(k, (), 1);
            r.trim(64, false);
        }
        assert!((0..8).all(|k| r.peek(&k).is_some()), "2 度使った物は残る");
    }

    #[test]
    fn a_stale_entry_is_not_a_hit() {
        let mut r: Residency<u64, u32> = Residency::default();
        r.insert(1, 7, 1);
        assert!(r.hit(&1, |v| *v == 8).is_none());
        assert_eq!(r.reused_bytes, 0);
        // 同じ鍵で作り直した物は上の段
        r.insert(1, 8, 1);
        assert_eq!(r.reused_bytes, 1);
    }
}
