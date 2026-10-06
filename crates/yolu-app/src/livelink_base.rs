//! Live Link で受けた元の絵（`MaterialOriginal`）を、新しく作ったテクスチャセットの一番下のレイヤー「元の絵」として入れる。
//!
//! 決まり:
//! - 入れるのは、Live Link のモデルを受けたときに**新しく作ったセット**と、**何も触っていない最初のセット**（開いた・保存したファイルが無く、
//!   描いていない・変えていない最初のプロジェクトの、最初のセット）だけ。描いたセット・利用者が開いたプロジェクトのセット・描いている
//!   最中のセットの文書には入れない（描いたものを黙って変えない）。受けるまでに文書が変わっていたら入れない（文書の ID と版で確かめる）。
//! - 来るはずの元の絵は、モデルのマテリアルの情報から決める: Color の流し込み先のプロパティに絵が入っているマテリアル。Unity は、
//!   そのスロットの元の絵を（絵が付かないときも様子だけ）必ず送る。そのセットは、元の絵が入るまで Unity に出さない（つないだ瞬間に、
//!   空の透明な絵で元の見た目を置き換えて、アバターを真っ黒にしない）。Unity から 30 秒届かなければ、元の絵なしで出す。
//! - 何も触っていない最初のセットは、元の絵の大きさ（新しく作るセットと同じ辺の丸め・上限。`sets::fit_side`）で作り直した文書へ入れる
//!   （4096 の絵が 2048 の最初のセットに縮まない）。作り直した文書に入らないとき（予算）は、今の文書へ縮めて入れる（印は拡大縮小）。
//!   新しく作ったセットは、作るときに Model の情報の大きさで作ってあるので作り直さない。新規プロジェクトの窓で解像度を選んで作った
//!   プロジェクト（`AppState::resolution_chosen`）の最初のセットも作り直さない（選んだ大きさのまま、元の絵を拡大縮小して入れる）。
//! - 絵の無いマテリアル（Model の情報に、Color の流し込み先のテクスチャの項目があり大きさが 0）は、不透明な白（Unity が絵の無いスロットを
//!   描く既定の白）の「元の絵」を入れる。Unity が絵を付けられなかったとき（読めない・大きすぎる・予算を超える）は白で埋めない
//!   （元の絵なしで出し、理由を知らせる）。層の欄の印は出さない（Unity が描くのと同じ値）。
//! - 絵の大きさがセットと違うときは、セットの大きさへ拡大縮小する（縮めは箱の平均、広げは双線形。アルファで重みを付け、透明な画素の
//!   RGB を混ぜない）。大きさが同じなら画素をそのまま入れる（透明な画素の RGB も）。リニアのテクスチャ（sRGB でない）は、Color の
//!   チャンネルが sRGB の画素を持つので、sRGB の画素へ直して入れる（見た目を変えない）。
//! - 一番下に足した層は初期化で、Undo の履歴には入れない（セットを作った直後の状態の一部。履歴は空のまま）。一番下の層の名前は「元の絵」。
//! - 層の欄の印: GPU を通して・圧縮から読んだ・リニアから直した・拡大縮小した絵のとき、理由をツールチップに出す（保存した .ylp には
//!   入らず、開き直すと印は無くなる。層は普通のピクセルレイヤー）。

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::time::{Duration, Instant};

use rayon::prelude::*;
use yolu_protocol::{
    channel, MaterialInfo, MaterialKey, MaterialOriginal, MaterialWant, Model, OriginalRead,
    OriginalState,
};

use crate::engine::{Channel, Document, LayerId, PixelClipboard};
use crate::lang::Lang;
use crate::model::ModelSource;
use crate::sets::fit_side;
use crate::state::AppState;

/// 元の絵が届く進みが止まってから、待つのをやめるまでの時間。
pub const STALL: Duration = Duration::from_secs(30);

/// 届いたが、まだ文書に入れていない元の絵の画素のバイトの合計の上限（描いている最中などで入れるのを待つあいだ）。
pub const MAX_PENDING_BYTES: u64 = 512 << 20;

/// 手元に残す元の絵（印の付いたもの）の画素のバイトの合計の上限。超えたら、使っていない順に捨てる。1 枚でこの上限を超える絵は残さない。
pub const MAX_CACHED_BYTES: u64 = 192 << 20;

/// 手元に残した元の絵 1 つ。
struct Cached {
    /// Unity が付けた絵の印（0 でない）。
    stamp: u64,
    /// 絵そのもの（世代とマテリアルの番号は、使うときに今のものへ替える）。
    original: MaterialOriginal,
    /// 最後に残した・使った順（大きいほど新しい）。
    used: u64,
}

/// 手元に残した元の絵（Unity が印を付けて送ってきたもの）。マテリアルの鍵（名前と GUID）とスロットで引く。モデルが替わっても・つなぎ直しても
/// 残す。Unity が頼みの `have` に付けた印と今の印が同じと答えたときだけ使い、印が違う・手元に無い・大きさが違うときは使わない
/// （古い絵を使う側には倒さない）。上限は [`MAX_CACHED_BYTES`]。
pub struct OriginalCache {
    entries: HashMap<(MaterialKey, String), Cached>,
    bytes: u64,
    limit: u64,
    tick: u64,
}

impl Default for OriginalCache {
    fn default() -> Self {
        OriginalCache::with_limit(MAX_CACHED_BYTES)
    }
}

impl OriginalCache {
    pub fn with_limit(limit: u64) -> OriginalCache {
        OriginalCache {
            entries: HashMap::new(),
            bytes: 0,
            limit,
            tick: 0,
        }
    }

    /// 手元の絵の印（頼みの `have`。無ければ 0）。
    pub fn have(&self, key: &MaterialKey, slot: &str) -> u64 {
        self.entries
            .get(&(key.clone(), slot.to_owned()))
            .map_or(0, |c| c.stamp)
    }

    /// 印の付いた、画素のある絵を手元に残す（同じ鍵・スロットの前の絵は置き換える）。印が 0・画素なしは残さず、前の絵も捨てる
    /// （印の無い新しい絵が来た以上、前の絵は今の絵ではない）。
    pub fn remember(&mut self, key: &MaterialKey, original: &MaterialOriginal) {
        let id = (key.clone(), original.slot.clone());
        if let Some(old) = self.entries.remove(&id) {
            self.bytes -= old.original.pixels.len() as u64;
        }
        let size = original.pixels.len() as u64;
        if original.state != OriginalState::Image || original.stamp == 0 || size > self.limit {
            return;
        }
        while self.bytes + size > self.limit {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, c)| c.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            if let Some(old) = self.entries.remove(&oldest) {
                self.bytes -= old.original.pixels.len() as u64;
            }
        }
        self.tick += 1;
        self.bytes += size;
        self.entries.insert(
            id,
            Cached {
                stamp: original.stamp,
                original: original.clone(),
                used: self.tick,
            },
        );
    }

    /// Unity が「印が同じ」と答えた絵を引く。印と大きさが手元の絵と合うときだけ（絵を使ったことにして、捨てる順を新しくする）。
    pub fn use_cached(
        &mut self,
        key: &MaterialKey,
        slot: &str,
        stamp: u64,
        size: (u32, u32),
    ) -> Option<MaterialOriginal> {
        let tick = self.tick + 1;
        let cached = self.entries.get_mut(&(key.clone(), slot.to_owned()))?;
        if stamp == 0
            || cached.stamp != stamp
            || (cached.original.width, cached.original.height) != size
        {
            return None;
        }
        self.tick = tick;
        cached.used = tick;
        Some(cached.original.clone())
    }

    /// 手元の絵の数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 手元の絵の画素のバイトの合計。
    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// 手元の絵を全部捨てる。
    pub fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

/// 層の欄に出す印 1 つ（入れた「元の絵」の層ごと。セッションの中だけで、保存しない）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginalMark {
    pub doc: u128,
    pub layer: LayerId,
    /// GPU を通して読んだ。
    pub gpu: bool,
    /// 圧縮されたテクスチャから読んだ（値は圧縮を解いたもの）。
    pub compressed: bool,
    /// リニアの絵を sRGB の画素へ直した。
    pub converted: bool,
    /// セットの大きさへ拡大縮小した（元の大きさ）。
    pub resized_from: Option<(u32, u32)>,
}

impl OriginalMark {
    /// 印を出すか（原本のそのままの値だけなら出さない）。
    pub fn is_noted(&self) -> bool {
        self.gpu || self.compressed || self.converted || self.resized_from.is_some()
    }

    /// ツールチップ（理由を 1 行ずつ）。
    pub fn tooltip(&self, lang: Lang, set: (u32, u32)) -> String {
        let mut lines = Vec::new();
        if self.gpu {
            lines.push(lang.pick(
                "GPU を通して読んだ値です".to_owned(),
                "Read through the GPU".to_owned(),
            ));
        }
        if self.compressed {
            lines.push(
                lang.pick(
                    "圧縮されたテクスチャから読んだ値です（元のファイルの値とは少し違います）"
                        .to_owned(),
                    "Read from a compressed texture (it differs slightly from the source file)"
                        .to_owned(),
                ),
            );
        }
        if self.converted {
            lines.push(lang.pick(
                "リニアのテクスチャを sRGB の画素へ直しています".to_owned(),
                "Converted from a linear texture to sRGB pixels".to_owned(),
            ));
        }
        if let Some((w, h)) = self.resized_from {
            lines.push(lang.pick(
                format!(
                    "{w}×{h} をセットの大きさ {}×{} に拡大縮小しています",
                    set.0, set.1
                ),
                format!("Scaled from {w}×{h} to the set size {}×{}", set.0, set.1),
            ));
        }
        lines.join("\n")
    }
}

/// 入れた「元の絵」の層の印の一覧（`AppState` が持つ）。
#[derive(Debug, Default)]
pub struct OriginalMarks(Vec<OriginalMark>);

impl OriginalMarks {
    /// 印の数の上限（古いものから捨てる）。
    const MAX: usize = 256;

    /// 文書 `doc` の層 `layer` の印（無ければ None）。
    pub fn get(&self, doc: u128, layer: LayerId) -> Option<&OriginalMark> {
        self.0.iter().find(|m| m.doc == doc && m.layer == layer)
    }

    fn push(&mut self, mark: OriginalMark) {
        if self.0.len() >= Self::MAX {
            self.0.remove(0);
        }
        self.0.push(mark);
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// 元の絵を待たせているセット 1 つ。
struct Wait {
    material: u32,
    /// マテリアルの鍵（手元の絵を引く）。
    key: MaterialKey,
    /// 来るはずのスロット（Color の流し込み先のプロパティ）。
    slot: String,
    /// Unity に頼んだ（頼みを出せるつながりで、頼んだ後。頼みは 1 度だけ。答えが無ければ [`STALL`] であきらめる）。
    requested: bool,
    /// 待たせ始めたときの文書の ID と版（変わっていたら入れない）。
    doc_id: u128,
    revision: u64,
    /// 何も触っていない最初のセット（元の絵の大きさで作り直してから入れてよい）。
    refit: bool,
    arrival: Option<Arrival>,
}

enum Arrival {
    /// 届いた（絵の付かない様子も）。
    Unity(MaterialOriginal),
    /// 届いたが、持てる量を超えるので持たなかった。
    TooMany,
    /// 絵の無いマテリアル: 来る絵は無く、白い絵で始める。
    White,
}

/// 受けた元の絵と、待たせているセット（`LiveLink` が持つ）。
pub struct LiveBase {
    generation: u32,
    waits: BTreeMap<u32, Wait>,
    /// 待ち始め、または最後に（どのマテリアルの）元の絵が届いた時刻。ここから [`STALL`] たっても届かなければ、あきらめる
    /// （1 枚ずつ届いているあいだは、全部で [`STALL`] を超えても待つ）。
    progress: Instant,
    /// 入れるのを待っている元の絵の画素のバイトの合計の上限（既定は [`MAX_PENDING_BYTES`]）。
    pending_limit: u64,
    /// 手元に残した元の絵（印の付いたもの）。つなぎ直しても残す。
    cache: OriginalCache,
    /// 次の `take_requests` で頼む、頼み直し（Unity の「印が同じ」の答えが手元の絵と合わなかったとき。印なしで頼む）。
    again: Vec<MaterialWant>,
}

impl Default for LiveBase {
    fn default() -> Self {
        LiveBase {
            generation: 0,
            waits: BTreeMap::new(),
            progress: Instant::now(),
            pending_limit: MAX_PENDING_BYTES,
            cache: OriginalCache::default(),
            again: Vec::new(),
        }
    }
}

/// Color の流し込み先のプロパティに絵が入っているなら、そのプロパティ（来るはずの元の絵のスロット）。
pub fn expected_slot(info: &MaterialInfo) -> Option<String> {
    let route = info.routes.iter().find(|r| r.channel == channel::COLOR)?;
    info.textures
        .iter()
        .any(|t| t.name == route.property && t.width > 0 && t.height > 0)
        .then(|| route.property.clone())
}

/// Color の流し込み先のプロパティに絵が無いと Unity が知らせているなら、そのプロパティ（白い元の絵のスロット）。Unity は、シェーダーの
/// テクスチャのプロパティを全部（絵が無ければ大きさ 0 で）知らせるので、項目があって大きさが 0 のときだけ。項目が無い・流し込み先が無い・
/// Color 以外のときは、絵が無いと言い切れないので白にしない。Unity が読めない・大きすぎる絵は、項目に大きさがあるので入らない。
pub fn white_slot(info: &MaterialInfo) -> Option<String> {
    let route = info.routes.iter().find(|r| r.channel == channel::COLOR)?;
    info.textures
        .iter()
        .any(|t| t.name == route.property && (t.width == 0 || t.height == 0))
        .then(|| route.property.clone())
}

impl LiveBase {
    /// モデルを受けた（セットの結び付けのあと）。`fresh` は、入れてよいセット（今回新しく作ったセットと、何も触っていない最初のセット）の
    /// uid で、`first` はそのうち何も触っていない最初のセット（元の絵の大きさで作り直してよい。ただし新規プロジェクトの窓で解像度を選んで
    /// 作ったプロジェクトは、選んだ大きさのまま元の絵を縮めて入れる）。前のモデルから待たせているセットのうち、
    /// 文書が変わっていないものは持ち越す。来る元の絵があるセットを待たせ、絵の無いマテリアルのセットは白い絵を入れるものとして待たせる
    /// （次の `poll` で入れて出す）。
    pub fn model(
        &mut self,
        state: &AppState,
        model: &Model,
        fresh: &[u32],
        first: Option<u32>,
        now: Instant,
    ) {
        self.generation = model.generation;
        let first = first.filter(|_| !state.resolution_chosen);
        let mut candidates: Vec<u32> = fresh.to_vec();
        candidates.extend(self.waits.keys().copied());
        candidates.sort_unstable();
        candidates.dedup();
        let mut next = BTreeMap::new();
        for uid in candidates {
            let Some(index) = state.sets.iter().position(|s| s.uid == uid) else {
                continue;
            };
            let set = state.sets.get(index).expect("位置を見た");
            let Some(material) = set.bound else {
                continue;
            };
            let Some(info) = model.materials.get(material as usize) else {
                continue;
            };
            let (slot, arrival) = match (expected_slot(info), white_slot(info)) {
                (Some(slot), _) => (slot, None),
                (None, Some(slot)) => (slot, Some(Arrival::White)),
                (None, None) => continue,
            };
            let doc = state.set_doc(index);
            let carried = self.waits.get(&uid);
            let (doc_id, revision) = match carried {
                Some(w) => (w.doc_id, w.revision),
                None => (doc.id(), doc.revision()),
            };
            if doc.id() != doc_id || doc.revision() != revision {
                continue;
            }
            next.insert(
                uid,
                Wait {
                    material,
                    key: info.key.clone(),
                    // 前のモデルから持ち越した待ちの頼みは、頼み直す（世代が替わったので、前の頼みへの答えは来ない）
                    requested: false,
                    slot,
                    doc_id,
                    revision,
                    refit: first == Some(uid) || carried.is_some_and(|w| w.refit),
                    arrival,
                },
            );
        }
        self.waits = next;
        self.progress = now;
    }

    /// Unity に頼む、元の絵（頼みを出せるつながりのときだけ呼ぶ）。元の絵を待たせているセットのマテリアルのうち、まだ頼んでいないもの
    /// （手元に印の付いた絵があれば `have` に付ける）と、手元の絵が合わなかったための頼み直し（印なし）。頼んだ待ちは頼み済みにする。
    /// 絵の無いマテリアル（白で始める）は、来る絵が無いので頼まない。
    pub fn take_requests(&mut self) -> Vec<MaterialWant> {
        let mut out = std::mem::take(&mut self.again);
        for wait in self.waits.values_mut() {
            if wait.requested || wait.arrival.is_some() {
                continue;
            }
            wait.requested = true;
            let have = self.cache.have(&wait.key, &wait.slot);
            out.push(MaterialWant::original(
                wait.material,
                wait.slot.clone(),
                have,
            ));
        }
        // 同じマテリアル・スロットは 1 つ（先の頼み。頼み直し（印なし）が先に並ぶ）
        let mut seen = std::collections::BTreeSet::new();
        out.retain(|w| seen.insert((w.material, w.slot.clone())));
        out
    }

    /// 手元に残した元の絵（試験・診断用）。
    pub fn cache(&self) -> &OriginalCache {
        &self.cache
    }

    /// このセットは、元の絵が入るまで Unity に出さない。
    pub fn holds(&self, uid: u32) -> bool {
        self.waits.contains_key(&uid)
    }

    /// 元の絵を待たせているセットがあるか。
    pub fn waiting(&self) -> bool {
        !self.waits.is_empty()
    }

    /// 待たせているセットの数（試験・診断用）。
    pub fn waiting_count(&self) -> usize {
        self.waits.len()
    }

    /// 元の絵を受けた。待たせていないマテリアルの絵は要らない（Ok で捨てる）。世代・スロットが合わない命令の食い違いは理由を返す
    /// （Unity への返事にする診断の文）。
    pub fn receive(&mut self, original: MaterialOriginal, now: Instant) -> Result<(), String> {
        if original.generation != self.generation {
            return Err(format!(
                "元の絵の世代 {} は今のモデルの世代 {} と違います",
                original.generation, self.generation
            ));
        }
        // 「印が同じ」の答え（画素なし）は、手元の絵と印・大きさが合えば、その絵が届いたものとして扱う。合わなければ使わず、
        // 印なしで頼み直す（手元の絵が無い・古い・別の大きさ。古い絵は使わない）
        let mut original = original;
        if original.state == OriginalState::Cached {
            let Some(key) = self
                .waits
                .values()
                .find(|w| w.material == original.material)
                .map(|w| w.key.clone())
            else {
                return Ok(());
            };
            match self.cache.use_cached(
                &key,
                &original.slot,
                original.stamp,
                (original.width, original.height),
            ) {
                Some(hit) => {
                    original = MaterialOriginal {
                        generation: original.generation,
                        material: original.material,
                        ..hit
                    }
                }
                None => {
                    self.again
                        .push(MaterialWant::original(original.material, original.slot, 0));
                    self.progress = now;
                    return Ok(());
                }
            }
        }
        let pending: u64 = self
            .waits
            .values()
            .filter_map(|w| match &w.arrival {
                Some(Arrival::Unity(o)) => Some(o.pixels.len() as u64),
                _ => None,
            })
            .sum();
        let Some(wait) = self
            .waits
            .values_mut()
            .find(|w| w.material == original.material)
        else {
            return Ok(());
        };
        if matches!(wait.arrival, Some(Arrival::White)) {
            // 絵が無いと知らせたマテリアルの元の絵は要らない（白で始める）
            return Ok(());
        }
        if wait.slot != original.slot {
            return Err(format!(
                "知らせていないスロットの元の絵です（マテリアル {}・{}）",
                original.material, original.slot
            ));
        }
        self.progress = now;
        wait.arrival = Some(
            if pending + original.pixels.len() as u64 > self.pending_limit {
                Arrival::TooMany
            } else {
                // 印の付いた絵は手元に残す（次に Unity が「印が同じ」と答えたとき使う）。印の無い絵は残さず、前の絵も捨てる
                let key = wait.key.clone();
                self.cache.remember(&key, &original);
                Arrival::Unity(original)
            },
        );
        Ok(())
    }

    /// 待たせているセットを全部出す（元の絵の命令を読めなかったとき。どの絵が欠けたか分からないので、待たない）。
    pub fn release_all(&mut self) {
        self.waits.clear();
    }

    /// つながりが終わった・モデルが替わった。
    pub fn clear(&mut self) {
        self.waits.clear();
        self.generation = 0;
    }

    /// 毎フレーム: 揃ったセットへ元の絵を入れ、進みが止まったものをあきらめる。入れなかった・入れられなかったものの理由を 1 つの知らせの文に
    /// まとめて返す。
    pub fn poll(&mut self, state: &mut AppState, session: u64, now: Instant) -> Option<String> {
        let ours = state.model.as_ref().is_some_and(|m| {
            m.source == (ModelSource::LiveLink { session }) && m.generation == self.generation
        });
        if !ours {
            self.waits.clear();
            return None;
        }
        let lang = state.lang;
        let mut failed: Vec<(String, String)> = Vec::new();
        for uid in self.waits.keys().copied().collect::<Vec<_>>() {
            let delivered = self.waits[&uid].arrival.is_some();
            let stalled = now.saturating_duration_since(self.progress) > STALL;
            if !delivered {
                if stalled {
                    self.waits.remove(&uid);
                    let name = set_name(state, uid);
                    failed.push((
                        name,
                        lang.pick("Unity から届きませんでした", "It did not arrive from Unity")
                            .to_owned(),
                    ));
                }
                continue;
            }
            // 描いている最中は文書を変えない（終わってから）
            if state.is_stroking() {
                continue;
            }
            let wait = self.waits.remove(&uid).expect("上で見た");
            if let Err(reason) = settle(state, uid, wait, lang) {
                failed.push((set_name(state, uid), reason));
            }
        }
        if failed.is_empty() {
            return None;
        }
        let list = failed
            .iter()
            .map(|(name, reason)| format!("{name}（{reason}）"))
            .collect::<Vec<_>>()
            .join(lang.pick("・", ", "));
        Some(lang.pick(
            format!("Live Link: 元の絵を入れませんでした — {list}"),
            format!("Live Link: Original not added — {list}"),
        ))
    }
}

fn set_name(state: &AppState, uid: u32) -> String {
    state
        .sets
        .iter()
        .find(|s| s.uid == uid)
        .map(|s| s.name.clone())
        .unwrap_or_default()
}

/// 揃ったセットの結果を決める: 絵が付いていれば（絵の無いマテリアルなら白を）文書の一番下に入れ、付いていなければ理由を返す。
fn settle(state: &mut AppState, uid: u32, wait: Wait, lang: Lang) -> Result<(), String> {
    let source = match wait.arrival {
        Some(Arrival::Unity(o)) => match o.state {
            OriginalState::Image => Source::Original(o),
            OriginalState::Unreadable | OriginalState::Cached => {
                return Err(lang
                    .pick("Unity が読めませんでした", "Unity could not read it")
                    .to_owned())
            }
            OriginalState::TooLarge => {
                return Err(lang.pick(
                    format!("大きすぎます（{}×{}）", o.width, o.height),
                    format!("Too large ({}×{})", o.width, o.height),
                ))
            }
            OriginalState::OverBudget => {
                return Err(lang
                    .pick(
                        "Unity が一度に送れる量を超えました",
                        "Over the amount Unity sends at once",
                    )
                    .to_owned())
            }
        },
        Some(Arrival::White) => Source::White,
        Some(Arrival::TooMany) => {
            return Err(lang
                .pick("受けた元の絵が多すぎます", "Too many received originals")
                .to_owned())
        }
        None => return Ok(()),
    };
    let Some(index) = state.sets.iter().position(|s| s.uid == uid) else {
        return Ok(());
    };
    let set = state.sets.get(index).expect("位置を見た");
    if set.bound != Some(wait.material) {
        return Err(lang
            .pick(
                "マテリアルから外れました",
                "No longer bound to the material",
            )
            .to_owned());
    }
    if let Some(reason) = &set.read_only {
        return Err(reason.clone());
    }
    let doc = state.set_doc(index);
    if doc.id() != wait.doc_id || doc.revision() != wait.revision {
        return Err(lang
            .pick("すでに編集されています", "Already edited")
            .to_owned());
    }
    match source {
        Source::Original(original) => install(state, index, original, wait.refit, lang),
        Source::White => install_white(state, index, lang),
    }
}

/// 入れるもの。
enum Source {
    Original(MaterialOriginal),
    White,
}

/// 元の絵の画素を `size` の大きさの sRGB の画素にする（大きさが違えば拡大縮小、リニアなら sRGB へ）。借りた画素は、大きさが同じときだけ写す
/// （大きな絵を、拡大縮小の前に丸ごと写さない）。
fn prepare(pixels: Cow<'_, [u8]>, from: (u32, u32), srgb: bool, size: (u32, u32)) -> Vec<u8> {
    let mut out = if from != size {
        resample(&pixels, [from.0, from.1], [size.0, size.1])
    } else {
        pixels.into_owned()
    };
    if !srgb {
        linear_to_srgb(&mut out);
    }
    out
}

/// 文書の大きさの画素を、一番下のレイヤー「元の絵」として入れる（履歴には入れない: 作った直後の初期化）。入れた層を返す。
/// 入らなければ文書は変えない。
fn add_bottom_layer(doc: &mut Document, pixels: Vec<u8>, lang: Lang) -> Result<LayerId, String> {
    let clip = PixelClipboard::from_image(doc.width(), doc.height(), pixels, Channel::Color)
        .map_err(|e| lang.core_error(&e))?;
    let name = lang.pick("元の絵", "Original");
    let pasted = doc
        .paste_as_layer(&clip, Channel::Color, Some(name), None)
        .map_err(|e| lang.core_error(&e))?;
    if let Err(e) = doc.move_layer(pasted.layer, 0) {
        let _ = doc.undo();
        return Err(lang.core_error(&e));
    }
    let _ = doc.clear_history();
    Ok(pasted.layer)
}

/// 元の絵を、セットの文書の一番下のレイヤーとして入れる（履歴には入れない）。`refit` は何も触っていない最初のセットで、元の絵の大きさで
/// 作り直した文書へ入れる（入らなければ今の文書へ、セットの大きさに縮めて入れる）。
fn install(
    state: &mut AppState,
    index: usize,
    original: MaterialOriginal,
    refit: bool,
    lang: Lang,
) -> Result<(), String> {
    let source = (original.width, original.height);
    let converted = !original.srgb;
    let (read, compressed) = (original.read, original.compressed);
    let mark = |doc: &Document, layer: LayerId| OriginalMark {
        doc: doc.id(),
        layer,
        gpu: read == OriginalRead::Gpu,
        compressed,
        converted,
        resized_from: (source != (doc.width(), doc.height())).then_some(source),
    };
    if refit {
        if let Some((doc, layer)) = rebuilt_with(state.set_doc(index), &original, lang) {
            let mark = mark(&doc, layer);
            state.swap_untouched_set_document(index, doc);
            state.link_originals.push(mark);
            return Ok(());
        }
    }
    let doc = state.set_doc_mut(index);
    let size = (doc.width(), doc.height());
    let pixels = prepare(Cow::Owned(original.pixels), source, original.srgb, size);
    let layer = add_bottom_layer(doc, pixels, lang)?;
    let mark = mark(state.set_doc(index), layer);
    state.link_originals.push(mark);
    Ok(())
}

/// `like`（何も触っていない最初のセットの文書）の代わりに、元の絵の大きさ（新しく作るセットと同じ辺の丸め・上限）で作り直した文書へ
/// 元の絵を入れたもの。大きさが同じなら作り直さず、作り直した文書に入らない（予算）ときも None（今の文書へ縮めて入れる）。
fn rebuilt_with(
    like: &Document,
    original: &MaterialOriginal,
    lang: Lang,
) -> Option<(Document, LayerId)> {
    let side = fit_side(original.width.max(original.height));
    if (side, side) == (like.width(), like.height()) {
        return None;
    }
    let mut doc = crate::newproject::new_set_document(
        side,
        side,
        like.tile_size(),
        lang,
        &crate::newproject::used_channels(like),
        like.normal_settings(),
    )
    .ok()?;
    // 予算は今の文書と同じ（入らなければ今の文書へ戻る）
    doc.set_minimum_undo_steps(like.minimum_undo_steps()).ok()?;
    doc.set_undo_budget_bytes(like.undo_budget_bytes()).ok()?;
    doc.set_stroke_budget_bytes(like.stroke_budget_bytes())
        .ok()?;
    doc.set_source_budget_bytes(like.source_budget_bytes())
        .ok()?;
    // 入らなかったときに今の文書へ入れ直せるよう、元の画素は手放さない（借りる）
    let pixels = prepare(
        Cow::Borrowed(&original.pixels),
        (original.width, original.height),
        original.srgb,
        (side, side),
    );
    let layer = add_bottom_layer(&mut doc, pixels, lang).ok()?;
    Some((doc, layer))
}

/// 絵の無いマテリアルのセットに、不透明な白の「元の絵」を入れる（Unity は絵の無いスロットを既定の白で描く。層の欄の印は付けない）。
fn install_white(state: &mut AppState, index: usize, lang: Lang) -> Result<(), String> {
    let doc = state.set_doc_mut(index);
    let pixels = vec![255u8; doc.width() as usize * doc.height() as usize * 4];
    add_bottom_layer(doc, pixels, lang).map(|_| ())
}

/// 軸 1 本の、出力の位置ごとの元の画素（先頭の位置と重み）。縮めは箱の平均、広げは双線形、同じ大きさは 1 対 1。
struct Taps {
    start: usize,
    weights: Vec<f32>,
}

fn axis(source: usize, target: usize) -> Vec<Taps> {
    (0..target)
        .map(|i| {
            if source == target {
                Taps {
                    start: i,
                    weights: vec![1.0],
                }
            } else if source > target {
                let (lo, hi) = (
                    i as f64 * source as f64 / target as f64,
                    (i + 1) as f64 * source as f64 / target as f64,
                );
                let first = lo.floor() as usize;
                let last = ((hi.ceil() as usize).min(source)).max(first + 1) - 1;
                let weights = (first..=last)
                    .map(|j| {
                        let overlap = hi.min((j + 1) as f64) - lo.max(j as f64);
                        (overlap.max(0.0) / (hi - lo)) as f32
                    })
                    .collect();
                Taps {
                    start: first,
                    weights,
                }
            } else {
                let c = (i as f64 + 0.5) * source as f64 / target as f64 - 0.5;
                if c <= 0.0 {
                    Taps {
                        start: 0,
                        weights: vec![1.0],
                    }
                } else if c >= (source - 1) as f64 {
                    Taps {
                        start: source - 1,
                        weights: vec![1.0],
                    }
                } else {
                    let a = c.floor();
                    let f = (c - a) as f32;
                    Taps {
                        start: a as usize,
                        weights: vec![1.0 - f, f],
                    }
                }
            }
        })
        .collect()
}

/// straight RGBA8（下の行が先）を `to` の大きさへ拡大縮小する。アルファで重みを付けて混ぜ（透明な画素の RGB は、周りが全部透明のときだけ
/// 使う）、同じ画素しか重ならない所はその画素のまま。
pub fn resample(src: &[u8], from: [u32; 2], to: [u32; 2]) -> Vec<u8> {
    let (sw, sh) = (from[0] as usize, from[1] as usize);
    let (dw, dh) = (to[0] as usize, to[1] as usize);
    let xs = axis(sw, dw);
    let ys = axis(sh, dh);
    let mut out = vec![0u8; dw * dh * 4];
    out.par_chunks_mut(dw * 4).enumerate().for_each(|(y, row)| {
        let ty = &ys[y];
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let tx = &xs[x];
            // 1 画素しか重ならない（同じ大きさ・端）ならそのまま
            if tx.weights.len() == 1 && ty.weights.len() == 1 {
                let at = ((ty.start * sw) + tx.start) * 4;
                px.copy_from_slice(&src[at..at + 4]);
                continue;
            }
            let (mut a, mut r, mut g, mut b) = (0f32, 0f32, 0f32, 0f32);
            let (mut zw, mut zr, mut zg, mut zb) = (0f32, 0f32, 0f32, 0f32);
            let mut first: Option<[u8; 4]> = None;
            let mut same = true;
            for (j, wy) in ty.weights.iter().enumerate() {
                for (i, wx) in tx.weights.iter().enumerate() {
                    let w = wy * wx;
                    if w <= 0.0 {
                        continue;
                    }
                    let at = (((ty.start + j) * sw) + tx.start + i) * 4;
                    let p = [src[at], src[at + 1], src[at + 2], src[at + 3]];
                    match first {
                        Some(f) if f != p => same = false,
                        None => first = Some(p),
                        _ => {}
                    }
                    if p[3] == 0 {
                        zw += w;
                        zr += w * p[0] as f32;
                        zg += w * p[1] as f32;
                        zb += w * p[2] as f32;
                        continue;
                    }
                    let k = w * p[3] as f32;
                    a += k;
                    r += k * p[0] as f32;
                    g += k * p[1] as f32;
                    b += k * p[2] as f32;
                }
            }
            if same {
                px.copy_from_slice(&first.unwrap_or([0; 4]));
                continue;
            }
            let byte = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            let alpha = byte(a);
            if alpha == 0 {
                if zw > 0.0 {
                    px.copy_from_slice(&[byte(zr / zw), byte(zg / zw), byte(zb / zw), 0]);
                }
                continue;
            }
            px.copy_from_slice(&[byte(r / a), byte(g / a), byte(b / a), alpha]);
        }
    });
    out
}

/// リニアの RGB（A はそのまま）を sRGB の画素へ直す。
pub fn linear_to_srgb(pixels: &mut [u8]) {
    let table: Vec<u8> = (0..=255u32)
        .map(|v| {
            let c = v as f64 / 255.0;
            let s = if c <= 0.003_130_8 {
                12.92 * c
            } else {
                1.055 * c.powf(1.0 / 2.4) - 0.055
            };
            (s * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    for p in pixels.as_chunks_mut::<4>().0 {
        p[0] = table[p[0] as usize];
        p[1] = table[p[1] as usize];
        p[2] = table[p[2] as usize];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_protocol::{ChannelRoute, MaterialKey, MeshData, Submesh, TextureProperty};

    fn info(name: &str, texture: bool) -> MaterialInfo {
        MaterialInfo {
            key: MaterialKey::Material {
                name: name.into(),
                asset: None,
            },
            shader: "Standard".into(),
            textures: if texture {
                vec![TextureProperty {
                    name: "_MainTex".into(),
                    width: 64,
                    height: 64,
                }]
            } else {
                vec![]
            },
            routes: vec![ChannelRoute {
                channel: channel::COLOR,
                property: "_MainTex".into(),
            }],
        }
    }

    fn model(generation: u32, materials: Vec<MaterialInfo>) -> Model {
        let n = materials.len() as u32;
        Model {
            generation,
            name: "試し".into(),
            materials,
            meshes: vec![MeshData {
                key: "0".into(),
                name: "Quad".into(),
                skinned: false,
                positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 0.0]],
                normals: vec![],
                uv0: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]],
                submeshes: (0..n)
                    .map(|m| Submesh {
                        material: m,
                        indices: vec![0, 2, 1],
                    })
                    .collect(),
            }],
        }
    }

    fn image(generation: u32, material: u32, bytes: usize) -> MaterialOriginal {
        MaterialOriginal {
            generation,
            material,
            slot: "_MainTex".into(),
            state: OriginalState::Image,
            read: OriginalRead::File,
            compressed: false,
            width: 1,
            height: bytes as u32 / 4,
            srgb: true,
            pixels: vec![255; bytes],
            stamp: 0,
        }
    }

    /// 絵が無いと知らせるマテリアル（Color の流し込み先のテクスチャの項目があり、大きさが 0）。
    fn empty(name: &str) -> MaterialInfo {
        let mut info = info(name, true);
        info.textures[0].width = 0;
        info.textures[0].height = 0;
        info
    }

    /// Live Link のモデルを受けた状態の AppState（セットは Body = 最初のセット・Hair = 新しいセット）と、待たせ始めた LiveBase。
    fn waiting() -> (AppState, LiveBase, Model, Instant) {
        waiting_with(vec![
            info("Body", true),
            info("Hair", true),
            info("Plain", false),
        ])
    }

    fn waiting_with(materials: Vec<MaterialInfo>) -> (AppState, LiveBase, Model, Instant) {
        let mut state = AppState::new(64, 64);
        let m = model(1, materials);
        let untouched = state.is_pristine().then(|| state.sets.current().uid);
        let (report, _) = state.receive_link_model(&m, 7);
        let mut fresh = report.created_sets.clone();
        fresh.extend(untouched);
        let mut base = LiveBase::default();
        let t0 = Instant::now();
        base.model(&state, &m, &fresh, untouched, t0);
        (state, base, m, t0)
    }

    #[test]
    fn only_a_color_slot_with_a_texture_is_waited_for() {
        assert_eq!(expected_slot(&info("A", true)).as_deref(), Some("_MainTex"));
        assert_eq!(
            expected_slot(&info("A", false)),
            None,
            "絵が入っていなければ来ない"
        );
        let mut no_route = info("A", true);
        no_route.routes.clear();
        assert_eq!(
            expected_slot(&no_route),
            None,
            "Color の流し込み先が無ければ来ない"
        );
        let mut other = info("A", true);
        other.routes[0].channel = channel::EMISSION;
        assert_eq!(
            expected_slot(&other),
            None,
            "Color 以外は送らない（Unity が元のまま見せる）"
        );
        let mut empty = info("A", true);
        empty.textures[0].width = 0;
        assert_eq!(expected_slot(&empty), None);
        let (state, base, _, _) = waiting();
        assert_eq!(base.waiting_count(), 2, "絵の無い Plain は待たせない");
        let uids: Vec<u32> = state.sets.iter().map(|s| s.uid).collect();
        assert!(base.holds(uids[0]) && base.holds(uids[1]) && !base.holds(uids[2]));
    }

    #[test]
    fn only_a_color_slot_that_unity_says_is_empty_gets_a_white_original() {
        assert_eq!(white_slot(&empty("A")).as_deref(), Some("_MainTex"));
        assert_eq!(white_slot(&info("A", true)), None, "絵があれば白にしない");
        assert_eq!(
            white_slot(&info("A", false)),
            None,
            "項目が無ければ、絵が無いと言い切れない"
        );
        let mut no_route = empty("A");
        no_route.routes.clear();
        assert_eq!(
            white_slot(&no_route),
            None,
            "Color の流し込み先が無ければ、描いた絵を見せない"
        );
        let mut other = empty("A");
        other.routes[0].channel = channel::EMISSION;
        assert_eq!(white_slot(&other), None, "Color 以外は触らない");
        // Unity は、幅か高さが 0 以下のテクスチャを絵が無いものとして元の絵を送らない
        let mut flat = info("A", true);
        flat.textures[0].height = 0;
        assert_eq!(white_slot(&flat).as_deref(), Some("_MainTex"));
        // 元の絵が来るものと白は重ならない
        assert_eq!(expected_slot(&empty("A")), None);
        assert_eq!(expected_slot(&flat), None);
    }

    #[test]
    fn a_set_gets_the_side_of_a_new_set_from_the_longest_side_of_the_picture() {
        for (longest, side) in [
            (1, 256),
            (100, 256),
            (256, 256),
            (257, 512),
            (600, 1024),
            (2048, 2048),
            (3000, 4096),
            (4096, 4096),
            (8192, 4096),
        ] {
            assert_eq!(fit_side(longest), side, "{longest}");
        }
        let mut material = info("A", true);
        material.textures[0].width = 600;
        material.textures[0].height = 300;
        assert_eq!(
            crate::sets::size_for(&material),
            fit_side(600),
            "新しいセットの大きさと同じ決め方"
        );
    }

    #[test]
    fn a_white_wait_needs_no_arrival_and_the_white_layer_goes_in_at_the_next_poll() {
        let (mut state, mut base, _, t0) = waiting_with(vec![empty("Body"), info("Hair", true)]);
        let uids: Vec<u32> = state.sets.iter().map(|s| s.uid).collect();
        assert_eq!(base.waiting_count(), 2);
        assert!(
            base.holds(uids[0]),
            "白が入るまで、空の絵で Unity に出さない"
        );
        // 絵が無いと知らせたマテリアルの元の絵が届いても、受けて捨てる（誤りにしない・白を置き換えない）
        let mut stray = image(1, 0, 4);
        stray.pixels = vec![1, 2, 3, 255];
        assert!(base.receive(stray, t0).is_ok());
        assert!(base.poll(&mut state, 7, t0).is_none());
        assert_eq!(base.waiting_count(), 1, "白は入れた。Hair は元の絵を待つ");
        assert!(!base.holds(uids[0]));
        let doc = state.set_doc(0);
        let names: Vec<&str> = doc.layers().iter().map(|l| l.name()).collect();
        assert_eq!(names, ["元の絵", "レイヤー 1"]);
        assert_eq!(
            crate::engine::layer_pixel(&doc.layers()[0], 0, 0),
            [255, 255, 255, 255]
        );
        assert_eq!(
            crate::engine::layer_pixel(&doc.layers()[0], 63, 63),
            [255, 255, 255, 255]
        );
        assert!(!doc.can_undo(), "初期化は履歴に入らない");
        assert_eq!(
            (doc.width(), doc.height()),
            (64, 64),
            "白は大きさを変えない"
        );
        assert!(state.link_originals.is_empty(), "層の欄の印は付けない");
    }

    #[test]
    fn a_white_original_that_does_not_fit_the_budget_is_declined_with_the_reason() {
        let (mut state, mut base, _, t0) = waiting_with(vec![empty("Body")]);
        state.doc.set_stroke_budget_bytes(10).unwrap();
        let text = base.poll(&mut state, 7, t0).unwrap();
        assert!(
            text.contains("元の絵を入れませんでした") && text.contains("予算"),
            "{text}"
        );
        assert!(
            !base.waiting(),
            "入れられなくても待ちは終わる（空のまま出す）"
        );
        assert_eq!(state.doc.layers().len(), 1);
    }

    #[test]
    fn a_silent_unity_stops_holding_the_sets_after_the_stall_and_says_which() {
        let (mut state, mut base, _, t0) = waiting();
        assert!(base
            .poll(&mut state, 7, t0 + STALL - Duration::from_secs(1))
            .is_none());
        assert_eq!(
            base.waiting_count(),
            2,
            "進みが止まって 30 秒たつまでは待つ"
        );
        // Hair が届く（入れて終わり）。Body の 30 秒は、最後に届いた時刻から数える
        base.receive(image(1, 1, 4), t0 + Duration::from_secs(10))
            .unwrap();
        assert!(base
            .poll(&mut state, 7, t0 + STALL + Duration::from_secs(1))
            .is_none());
        assert_eq!(
            base.waiting_count(),
            1,
            "Hair は入れた。Body は最後の到着から 21 秒なので、まだ待つ"
        );
        let text = base
            .poll(
                &mut state,
                7,
                t0 + Duration::from_secs(10) + STALL + Duration::from_secs(1),
            )
            .unwrap();
        assert!(
            text.contains("Body") && text.contains("Unity から届きませんでした"),
            "{text}"
        );
        assert!(
            !text.contains("Hair"),
            "届いたセットは理由に挙げない: {text}"
        );
        assert_eq!(base.waiting_count(), 0, "出す（待ちをやめる）");
    }

    #[test]
    fn originals_arriving_one_by_one_keep_the_others_waiting_beyond_the_stall_in_total() {
        let (mut state, mut base, _, t0) = waiting();
        // 1 枚目が 20 秒で、2 枚目が 40 秒で届く（全部で 30 秒を超えても、止まってはいない）
        let mut body = image(1, 0, 4);
        body.state = OriginalState::Unreadable;
        body.pixels.clear();
        base.receive(body, t0 + Duration::from_secs(20)).unwrap();
        let text = base
            .poll(&mut state, 7, t0 + Duration::from_secs(40))
            .unwrap();
        assert!(text.contains("Body") && !text.contains("Hair"), "{text}");
        assert_eq!(
            base.waiting_count(),
            1,
            "Hair はまだ待つ（最後の到着から 20 秒）"
        );
        base.receive(image(1, 1, 4), t0 + Duration::from_secs(40))
            .unwrap();
        assert!(base
            .poll(&mut state, 7, t0 + Duration::from_secs(40))
            .is_none());
        assert_eq!(base.waiting_count(), 0, "届いて入れた");
    }

    #[test]
    fn a_command_that_cannot_be_read_lets_every_set_out() {
        let (_, mut base, _, _) = waiting();
        base.release_all();
        assert!(!base.waiting());
    }

    #[test]
    fn originals_of_another_generation_or_slot_are_refused_and_unwaited_ones_are_ignored() {
        let (_, mut base, _, t0) = waiting();
        assert!(base
            .receive(image(2, 0, 4), t0)
            .unwrap_err()
            .contains("世代"));
        let mut slot = image(1, 0, 4);
        slot.slot = "_BaseMap".into();
        assert!(base.receive(slot, t0).unwrap_err().contains("_BaseMap"));
        // 待たせていないマテリアル（絵の無い Plain・モデルに無い番号）は黙って捨てる
        assert!(base.receive(image(1, 2, 4), t0).is_ok());
        assert!(base.receive(image(1, 9, 4), t0).is_ok());
        assert_eq!(base.waiting_count(), 2);
    }

    #[test]
    fn originals_that_wait_to_be_added_are_limited_and_the_rest_is_declined_with_a_reason() {
        let (mut state, mut base, _, t0) = waiting();
        base.pending_limit = 12;
        base.receive(image(1, 0, 8), t0).unwrap();
        base.receive(image(1, 1, 8), t0).unwrap();
        // 描いている最中は入れずに待つ（上限を超えた 2 つ目は、持たずに理由で出す）
        let text = base.poll(&mut state, 7, t0).unwrap();
        assert!(
            text.contains("Hair") && text.contains("多すぎます"),
            "{text}"
        );
        assert!(!text.contains("Body"), "{text}");
    }

    #[test]
    fn waits_end_when_the_model_is_not_the_links_any_more() {
        let (mut state, mut base, _, t0) = waiting();
        state.close_link_model(1);
        assert!(base.poll(&mut state, 7, t0).is_none());
        assert!(!base.waiting(), "モデルが閉じたら待たない（セットは出す）");
        let (mut state, mut base, _, t0) = waiting();
        assert!(
            base.poll(&mut state, 8, t0).is_none(),
            "別のつながりのモデル"
        );
        assert!(!base.waiting());
    }

    fn solid(w: usize, h: usize, p: [u8; 4]) -> Vec<u8> {
        p.repeat(w * h)
    }

    #[test]
    fn the_same_size_keeps_every_pixel_even_the_rgb_of_transparent_ones() {
        let src: Vec<u8> = (0..4 * 4 * 4).map(|i| (i * 7 % 256) as u8).collect();
        assert_eq!(resample(&src, [4, 4], [4, 4]), src);
    }

    #[test]
    fn shrinking_averages_boxes_and_a_flat_area_stays_exact() {
        // 2 × 2 → 1 × 1: 平均（不透明）
        let src = [
            [0, 0, 0, 255],
            [100, 0, 0, 255],
            [0, 100, 0, 255],
            [100, 100, 0, 255],
        ]
        .concat();
        assert_eq!(resample(&src, [2, 2], [1, 1]), [50, 50, 0, 255]);
        // 一様な絵は縮めても丸め誤差を出さない
        let flat = solid(8, 8, [13, 77, 201, 255]);
        assert_eq!(
            resample(&flat, [8, 8], [3, 5]),
            solid(3, 5, [13, 77, 201, 255])
        );
        // 非正方形: 4 × 2 → 2 × 2（横だけ縮める）
        let row: Vec<u8> = [
            [10, 0, 0, 255],
            [30, 0, 0, 255],
            [50, 0, 0, 255],
            [70, 0, 0, 255],
        ]
        .concat()
        .repeat(2);
        let out = resample(&row, [4, 2], [2, 2]);
        assert_eq!(out[..4], [20, 0, 0, 255]);
        assert_eq!(out[4..8], [60, 0, 0, 255]);
    }

    #[test]
    fn transparent_pixels_do_not_darken_their_opaque_neighbours() {
        // 不透明な赤と、RGB が黒の透明な画素 → 縮めた画素は半分の透明の赤（黒を混ぜない）
        let src = [[255, 0, 0, 255], [0, 0, 0, 0]].concat();
        assert_eq!(resample(&src, [2, 1], [1, 1]), [255, 0, 0, 128]);
        // 全部が透明なら、透明な画素の RGB の平均
        let src = [[10, 20, 30, 0], [30, 40, 50, 0]].concat();
        assert_eq!(resample(&src, [2, 1], [1, 1]), [20, 30, 40, 0]);
    }

    #[test]
    fn enlarging_is_bilinear_and_keeps_the_corners() {
        let src = [[0, 0, 0, 255], [200, 0, 0, 255]].concat();
        let out = resample(&src, [2, 1], [4, 1]);
        assert_eq!(out[..4], [0, 0, 0, 255], "左端は元の画素");
        assert_eq!(out[12..], [200, 0, 0, 255], "右端は元の画素");
        // 間は単調に増える
        let reds: Vec<u8> = out.as_chunks::<4>().0.iter().map(|p| p[0]).collect();
        assert!(reds.windows(2).all(|w| w[0] <= w[1]), "{reds:?}");
        assert!(reds[1] > 0 && reds[2] < 200);
    }

    #[test]
    fn linear_pixels_become_srgb_pixels_and_alpha_is_untouched() {
        let mut px = [0, 0, 0, 7, 255, 255, 255, 255, 55, 55, 55, 128];
        linear_to_srgb(&mut px);
        assert_eq!(px[..8], [0, 0, 0, 7, 255, 255, 255, 255]);
        // リニア 55/255 ≈ 0.2157 → sRGB ≈ 0.5017 → 128
        assert_eq!(px[8..11], [128, 128, 128]);
        assert_eq!(px[11], 128);
    }

    #[test]
    fn the_mark_is_shown_only_for_a_value_that_is_not_the_original_files() {
        let mark = |gpu, compressed, converted, resized_from| OriginalMark {
            doc: 1,
            layer: LayerId(1),
            gpu,
            compressed,
            converted,
            resized_from,
        };
        assert!(!mark(false, false, false, None).is_noted());
        assert!(mark(true, false, false, None).is_noted());
        assert!(mark(false, true, false, None).is_noted());
        assert!(mark(false, false, true, None).is_noted());
        assert!(mark(false, false, false, Some((4096, 2048))).is_noted());
        let tip = mark(true, true, false, Some((4096, 2048))).tooltip(Lang::En, (2048, 2048));
        assert_eq!(tip.lines().count(), 3);
        assert!(tip.contains("4096×2048") && tip.contains("2048×2048"));
        assert!(!tip.chars().any(|c| ('\u{3040}'..='\u{9fff}').contains(&c)));
    }

    fn key(name: &str) -> MaterialKey {
        MaterialKey::Material {
            name: name.into(),
            asset: None,
        }
    }

    fn stamped(generation: u32, material: u32, bytes: usize, stamp: u64) -> MaterialOriginal {
        MaterialOriginal {
            stamp,
            ..image(generation, material, bytes)
        }
    }

    fn cached_answer(generation: u32, material: u32, bytes: usize, stamp: u64) -> MaterialOriginal {
        MaterialOriginal {
            state: OriginalState::Cached,
            pixels: Vec::new(),
            stamp,
            ..image(generation, material, bytes)
        }
    }

    #[test]
    fn the_cache_keeps_stamped_pictures_and_uses_one_only_when_the_stamp_and_size_agree() {
        let mut cache = OriginalCache::default();
        assert!(cache.is_empty());
        let body = stamped(1, 0, 16, 11);
        cache.remember(&key("Body"), &body);
        assert_eq!(
            (
                cache.len(),
                cache.bytes(),
                cache.have(&key("Body"), "_MainTex")
            ),
            (1, 16, 11)
        );
        assert_eq!(
            cache.have(&key("Hair"), "_MainTex"),
            0,
            "別のマテリアルの絵は持たない"
        );
        assert_eq!(
            cache.have(&key("Body"), "_BaseMap"),
            0,
            "別のスロットの絵は持たない"
        );
        // 印と大きさが合えば使える（画素はそのまま）。1 つでも違えば使えない
        let hit = cache
            .use_cached(&key("Body"), "_MainTex", 11, (1, 4))
            .unwrap();
        assert_eq!(hit.pixels, body.pixels);
        assert!(
            cache
                .use_cached(&key("Body"), "_MainTex", 12, (1, 4))
                .is_none(),
            "印が違う"
        );
        assert!(
            cache
                .use_cached(&key("Body"), "_MainTex", 0, (1, 4))
                .is_none(),
            "印なしは使わない"
        );
        assert!(
            cache
                .use_cached(&key("Body"), "_MainTex", 11, (2, 2))
                .is_none(),
            "大きさが違う"
        );
        assert!(cache
            .use_cached(&key("Hair"), "_MainTex", 11, (1, 4))
            .is_none());
        // 印の無い絵・絵の付かない様子は残さず、前の絵も捨てる（印の無い新しい絵が来た以上、前の絵は今の絵ではない）
        cache.remember(&key("Body"), &stamped(1, 0, 16, 0));
        assert!(cache.is_empty());
        assert_eq!(cache.bytes(), 0);
        cache.remember(&key("Body"), &body);
        let mut declined = cached_answer(1, 0, 16, 5);
        declined.state = OriginalState::Unreadable;
        cache.remember(&key("Body"), &declined);
        assert!(cache.is_empty(), "読めなかった絵の後に、前の絵を使わない");
        // 同じ鍵・スロットは置き換える
        cache.remember(&key("Body"), &stamped(1, 0, 16, 1));
        cache.remember(&key("Body"), &stamped(1, 0, 8, 2));
        assert_eq!(
            (
                cache.len(),
                cache.bytes(),
                cache.have(&key("Body"), "_MainTex")
            ),
            (1, 8, 2)
        );
    }

    #[test]
    fn the_cache_is_bounded_and_drops_the_least_recently_used_picture() {
        let mut cache = OriginalCache::with_limit(40);
        cache.remember(&key("A"), &stamped(1, 0, 16, 1));
        cache.remember(&key("B"), &stamped(1, 0, 16, 2));
        // A を使うと、捨てる順は B が先になる
        assert!(cache.use_cached(&key("A"), "_MainTex", 1, (1, 4)).is_some());
        cache.remember(&key("C"), &stamped(1, 0, 16, 3));
        assert_eq!(cache.bytes(), 32);
        assert_eq!(cache.have(&key("A"), "_MainTex"), 1);
        assert_eq!(
            cache.have(&key("B"), "_MainTex"),
            0,
            "使っていない B を捨てた"
        );
        assert_eq!(cache.have(&key("C"), "_MainTex"), 3);
        // 1 枚で上限を超える絵は残さない（ほかの絵も追い出さない）
        cache.remember(&key("D"), &stamped(1, 0, 44, 4));
        assert_eq!((cache.len(), cache.bytes()), (2, 32));
        assert_eq!(cache.have(&key("D"), "_MainTex"), 0);
        cache.clear();
        assert!(cache.is_empty() && cache.bytes() == 0);
    }

    #[test]
    fn the_sets_that_wait_are_asked_for_once_with_the_stamp_the_standalone_holds() {
        let (_, mut base, _, _) = waiting();
        // 最初は手元に何も無い: 待たせた 2 つ（絵の無い Plain は頼まない）を、印なしで頼む
        let asked = base.take_requests();
        assert_eq!(
            asked,
            vec![
                MaterialWant::original(0, "_MainTex", 0),
                MaterialWant::original(1, "_MainTex", 0)
            ]
        );
        assert!(
            base.take_requests().is_empty(),
            "頼んだものは、もう頼まない"
        );
        // 手元に Body の絵がある状態で、モデルを送り直された: Body の頼みには印が付き、Hair は印なし
        let (state, mut base, m, t0) = waiting();
        base.cache
            .remember(&key("Body"), &stamped(1, 0, 16, 0xABCD));
        let uids: Vec<u32> = state.sets.iter().map(|s| s.uid).collect();
        base.model(&state, &m, &uids[..2], None, t0);
        let asked = base.take_requests();
        assert_eq!(
            asked,
            vec![
                MaterialWant::original(0, "_MainTex", 0xABCD),
                MaterialWant::original(1, "_MainTex", 0)
            ]
        );
        // 次のモデルの送り直しでは、持ち越した待ちも頼み直す（世代が替わって、前の頼みへの答えは来ない）
        base.model(&state, &m, &[], None, t0);
        assert_eq!(base.take_requests().len(), 2);
    }

    #[test]
    fn a_cached_answer_with_the_same_stamp_uses_the_picture_in_hand() {
        let (mut state, mut base, _, t0) = waiting();
        base.cache.remember(&key("Body"), &stamped(1, 0, 16, 7));
        let hand = base
            .cache
            .use_cached(&key("Body"), "_MainTex", 7, (1, 4))
            .unwrap()
            .pixels;
        base.take_requests();
        // Unity の答え: 印が同じ（画素なし）。手元の絵が届いたものとして扱い、セットに入る
        base.receive(cached_answer(1, 0, 16, 7), t0).unwrap();
        assert!(base.take_requests().is_empty(), "頼み直さない");
        base.receive(image(1, 1, 4), t0).unwrap();
        assert!(base.poll(&mut state, 7, t0).is_none());
        assert_eq!(base.waiting_count(), 0, "手元の絵で、待ちが終わった");
        let doc = state.set_doc(0);
        assert_eq!(
            doc.layers().iter().map(|l| l.name()).collect::<Vec<_>>(),
            ["元の絵", "レイヤー 1"]
        );
        // 入った画素は手元の絵の画素（1 × 4 の絵を 64 の文書の大きさへ拡大したもの。端は元の画素）
        let bottom = crate::engine::layer_pixel(&doc.layers()[0], 0, 0);
        assert_eq!(bottom, [hand[0], hand[1], hand[2], hand[3]]);
    }

    #[test]
    fn a_cached_answer_that_does_not_match_is_not_used_and_the_picture_is_asked_for_again() {
        for (stamp, size_bytes) in [(8u64, 16usize), (7, 32)] {
            let (mut state, mut base, _, t0) = waiting();
            base.cache.remember(&key("Body"), &stamped(1, 0, 16, 7));
            base.take_requests();
            // 印が違う（Unity の絵が変わった）・大きさが違う: 手元の絵は使わない
            base.receive(
                cached_answer(1, 0, size_bytes, stamp),
                t0 + Duration::from_secs(1),
            )
            .unwrap();
            assert!(base
                .poll(&mut state, 7, t0 + Duration::from_secs(1))
                .is_none());
            assert_eq!(
                base.waiting_count(),
                2,
                "古い絵で入れず、待ち続ける（stamp {stamp}）"
            );
            assert_eq!(
                base.take_requests(),
                vec![MaterialWant::original(0, "_MainTex", 0)],
                "印なしで頼み直す（stamp {stamp}）"
            );
            assert!(base.take_requests().is_empty());
            // 頼み直しの答え（画素つき・新しい印）で入り、手元の絵も新しくなる
            base.receive(stamped(1, 0, 32, 99), t0 + Duration::from_secs(2))
                .unwrap();
            assert_eq!(base.cache().have(&key("Body"), "_MainTex"), 99);
        }
        // 手元に何も無いのに「印が同じ」と言われても、使わず頼み直す
        let (_, mut base, _, t0) = waiting();
        base.take_requests();
        base.receive(cached_answer(1, 1, 16, 5), t0).unwrap();
        assert_eq!(
            base.take_requests(),
            vec![MaterialWant::original(1, "_MainTex", 0)]
        );
        // 待たせていないマテリアルの答えは、受けて捨てる（頼み直さない）
        base.receive(cached_answer(1, 2, 16, 5), t0).unwrap();
        assert!(base.take_requests().is_empty());
    }

    #[test]
    fn a_picture_without_a_stamp_is_never_kept_and_a_stamped_one_is_kept_once_it_arrives() {
        let (_, mut base, _, t0) = waiting();
        base.receive(image(1, 0, 16), t0).unwrap();
        assert!(base.cache().is_empty(), "印の無い絵は手元に残さない");
        base.receive(stamped(1, 1, 16, 3), t0).unwrap();
        assert_eq!(base.cache().have(&key("Hair"), "_MainTex"), 3);
        // つなぎが終わっても手元の絵は残る（つなぎ直しで使う）
        base.clear();
        assert_eq!(base.cache().len(), 1);
    }
}
