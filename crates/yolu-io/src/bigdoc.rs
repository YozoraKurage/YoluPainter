//! 大きな正本: 正本の版 26（分けた正本）と、正本を全体の `Vec` に組まずに流して作る・読む道。
//!
//! 版 26 は中身の正本（中の版 21〜25 の並び）が [`Thresholds::split_above`]（512 MiB）を超える文書だけが使う。
//! `document.utpaint` は `DOTPAINT`・26・中の版・部分の数・中の版の並びから `Bytes` の値（色・画素）を抜いたもの（ヘッダー）、
//! `Bytes` の値は並びの順に同じセットの `document.utpaint.1`・`.2`…（部分）へ書く。部分の区切りは、2 番目からの層の始まりと、
//! [`Thresholds::part_bytes`]（256 MiB）を超える手前（値 1 つは分けない）。ただし今の部分も次の層の値も [`Thresholds::part_min`]（16 MiB）に
//! 満たなければ、層の始まりで区切らずに続ける（層の多い文書でエントリが増えすぎないように）。変わらない層の部分は前と同じ中身になり、
//! 復旧の世代で共有される。読み手は区切りの位置を決め打ちせず、値が部分の境目をまたがないこと・空の部分が無いこと・部分の数と余りが
//! 合うことを確かめる。
//!
//! メモリ: 作る・読む・確かめるどの道も、持つのは層 1 枚ぶんの並びと小さな作業域だけ（core の文書とその写しのほかに）。
use crate::{
    check, check_budget,
    core_bridge::{check_writable, version_of, write_document, CoreLoad, Fields, Sink},
    native::{ByteSource, Keep, Parse, Part, PartStream, StreamSource, SPLIT_VERSION},
    package::{Blob, Made, Thresholds, MAX_ONE_ENTRY},
    Error, NativeDocument, NativeField, NativeValue, Result,
};
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    io::Write,
    rc::Rc,
    sync::{Arc, OnceLock},
};
use yolu_core::{Document, Surface, TileCoord};

/// メモリの正本（`NativeDocument`）にできる大きさを超えた（大きな正本は流して読む: `to_core`）。
const TOO_BIG_FOR_MEMORY: &str = "正本が大きすぎてメモリに読めません";

/// 部分の区切り: `Bytes` の値の流れのバイトの範囲 `[始まり, 終わり)`。
pub(crate) type Ranges = Vec<(u64, u64)>;

/// 値の並び（`(層の番号, 長さ)`。層より前の値は層 0 に数える）から、部分の区切りを決める。
pub(crate) fn cut(values: &[(u32, u32)], t: &Thresholds) -> Ranges {
    let mut totals: Vec<u64> = Vec::new();
    for &(section, len) in values {
        let s = section as usize;
        if totals.len() <= s {
            totals.resize(s + 1, 0);
        }
        totals[s] += u64::from(len);
    }
    let mut ranges = Vec::new();
    let (mut start, mut at, mut current) = (0u64, 0u64, None::<u32>);
    for &(section, len) in values {
        let len = u64::from(len);
        let size = at - start;
        if size > 0 {
            let new_layer = current != Some(section) && section >= 1;
            let big = size >= t.part_min || totals[section as usize] >= t.part_min;
            if (new_layer && big) || size + len > t.part_bytes {
                ranges.push((start, at));
                start = at;
            }
        }
        at += len;
        current = Some(section);
    }
    if at > start {
        ranges.push((start, at));
    }
    ranges
}

// ───────── core の文書から作る正本 ─────────

/// 正本の形（作る前に数える。画素は写さない）。
#[derive(Debug)]
pub(crate) struct DocPlan {
    /// 中の版。
    pub version: i32,
    /// 平の値のバイト数（識別子と版を含む）。
    pub plain: u64,
    /// `Bytes` の値のバイト数。
    pub values: u64,
    /// 分けるなら部分の区切り。
    pub split: Option<Ranges>,
}
impl DocPlan {
    /// 分けない正本の長さ。
    pub fn total(&self) -> u64 {
        self.plain + self.values
    }
    /// 版 26 のヘッダーの長さ（識別子 8・26・中の版・部分の数・識別子と版の後ろの平の値）。
    pub fn header_len(&self) -> u64 {
        self.plain + 8
    }
}
/// 形を数える書き先。
struct PlanSink {
    plain: u64,
    values: Vec<(u32, u32)>,
    section: u32,
}
impl Sink for PlanSink {
    fn plain(&mut self, b: &[u8]) -> Result<()> {
        self.plain += b.len() as u64;
        Ok(())
    }
    fn value(&mut self, b: &[u8]) -> Result<()> {
        self.values.push((self.section, b.len() as u32));
        Ok(())
    }
    fn tile(&mut self, surface: &Surface, _: TileCoord) -> Result<()> {
        self.values
            .push((self.section, surface.tile_bytes() as u32));
        Ok(())
    }
    fn layer(&mut self, i: usize) -> Result<()> {
        self.section = i as u32;
        Ok(())
    }
}

/// core の文書（保存・書き置きのための写し）から作る正本。形は作るときに数え、SHA-256 と確かめ（読み手で読み直す）は初めて要るときに
/// 1 回だけ流して数える。
pub(crate) struct CoreDoc {
    doc: Arc<Document>,
    plan: DocPlan,
    checked: OnceLock<std::result::Result<Checked, String>>,
}
/// 1 回目に流して数えたもの。
struct Checked {
    /// 分けない正本の SHA-256（分けるときは None）。
    full: Option<String>,
    /// 版 26 のヘッダーと部分の SHA-256。
    header: Option<String>,
    parts: Vec<String>,
    /// 読み手で読み直した骨組み。
    skeleton: Arc<NativeDocument>,
}
impl std::fmt::Debug for CoreDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreDoc").field("plan", &self.plan).finish()
    }
}
/// 作る正本のどのエントリか。
#[derive(Clone, Copy, Debug)]
enum Which {
    Full,
    Header,
    Part(usize),
}
impl CoreDoc {
    /// 書けるかを確かめ、形を数える（画素は写さない）。
    pub fn new(doc: Arc<Document>) -> Result<Arc<Self>> {
        check_writable(&doc)?;
        let version = version_of(&doc);
        let mut sink = PlanSink {
            plain: 0,
            values: Vec::new(),
            section: 0,
        };
        write_document(&mut sink, &doc, version)?;
        let t = Thresholds::current();
        let total = sink.plain + sink.values.iter().map(|&(_, n)| u64::from(n)).sum::<u64>();
        let values = total - sink.plain;
        let split = (total > t.split_above).then(|| cut(&sink.values, &t));
        Ok(Arc::new(Self {
            doc,
            plan: DocPlan {
                version,
                plain: sink.plain,
                values,
                split,
            },
            checked: OnceLock::new(),
        }))
    }
    pub fn plan(&self) -> &DocPlan {
        &self.plan
    }
    /// セットのエントリ（`prefix` は `sets/<ID>/`）。分けない正本は `document.utpaint` だけ、分けるならヘッダーと部分。
    pub fn entries(self: &Arc<Self>, prefix: &str) -> Vec<(String, Blob)> {
        let blob = |which| -> Blob {
            Blob::made(Arc::new(Entry {
                doc: self.clone(),
                which,
            }))
        };
        match &self.plan.split {
            None => vec![(format!("{prefix}document.utpaint"), blob(Which::Full))],
            Some(ranges) => {
                std::iter::once((format!("{prefix}document.utpaint"), blob(Which::Header)))
                    .chain((0..ranges.len()).map(|k| {
                        (
                            format!("{prefix}document.utpaint.{}", k + 1),
                            blob(Which::Part(k)),
                        )
                    }))
                    .collect()
            }
        }
    }
    fn len_of(&self, which: Which) -> u64 {
        match (which, &self.plan.split) {
            (Which::Full, _) => self.plan.total(),
            (Which::Header, _) => self.plan.header_len(),
            (Which::Part(k), Some(r)) => r[k].1 - r[k].0,
            (Which::Part(_), None) => 0,
        }
    }
    /// 1 回目: 流して SHA-256 を数え、読み手で読み直して骨組みを作る。
    fn checked(&self) -> Result<&Checked> {
        self.checked
            .get_or_init(|| self.check_once().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| Error::InvalidData(format!("正本を作れません: {e}")))
    }
    fn check_once(&self) -> Result<Checked> {
        let split = self.plan.split.as_ref();
        let mut hashes = Hashes {
            full: split.is_none().then(Sha256::new),
            header: split.map(|r| {
                let mut h = Sha256::new();
                h.update(b"DOTPAINT");
                h.update(SPLIT_VERSION.to_le_bytes());
                h.update(self.plan.version.to_le_bytes());
                h.update((r.len() as i32).to_le_bytes());
                h
            }),
            parts: split.map_or_else(Vec::new, |r| r.iter().map(|_| Sha256::new()).collect()),
            ranges: split.cloned().unwrap_or_default(),
            plain_at: 0,
            value_at: 0,
        };
        let skeleton = walk(
            &self.doc,
            self.plan.version,
            Keep::Skeleton,
            &mut hashes,
            |_, _| Ok(()),
        )?;
        Ok(Checked {
            full: hashes.full.map(|h| format!("{:x}", h.finalize())),
            header: hashes.header.map(|h| format!("{:x}", h.finalize())),
            parts: hashes
                .parts
                .into_iter()
                .map(|h| format!("{:x}", h.finalize()))
                .collect(),
            skeleton: Arc::new(skeleton),
        })
    }
    pub fn skeleton(&self) -> Result<Arc<NativeDocument>> {
        Ok(self.checked()?.skeleton.clone())
    }
    fn sha_of(&self, which: Which) -> Result<String> {
        let c = self.checked()?;
        Ok(match which {
            Which::Full => c.full.clone(),
            Which::Header => c.header.clone(),
            Which::Part(k) => c.parts.get(k).cloned(),
        }
        .expect("形と同じエントリ"))
    }
    /// エントリの中身を `out` へ書く（画素は要る部分の分だけ写す）。
    fn write_entry(&self, which: Which, out: &mut dyn Write) -> Result<()> {
        let mut sink = EntrySink {
            out,
            which,
            ranges: self.plan.split.as_deref().unwrap_or(&[]),
            plain_at: 0,
            value_at: 0,
            tile: Vec::new(),
        };
        if let Which::Header = which {
            let parts = self.plan.split.as_ref().map_or(0, Vec::len) as i32;
            sink.out.write_all(b"DOTPAINT")?;
            sink.out.write_all(&SPLIT_VERSION.to_le_bytes())?;
            sink.out.write_all(&self.plan.version.to_le_bytes())?;
            sink.out.write_all(&parts.to_le_bytes())?;
        }
        write_document(&mut sink, &self.doc, self.plan.version)
    }
    /// 流して core の文書にする（作った正本を読み手で読むのと同じ。持つのは層 1 枚ぶん）。
    pub fn to_core(&self, budget: Option<u64>) -> Result<Document> {
        let skeleton = self.skeleton()?;
        refuse_issues(&skeleton)?;
        let mut load: Option<CoreLoad> = None;
        let mut head: Vec<NativeField> = Vec::new();
        let version = self.plan.version;
        let size = (skeleton.width(), skeleton.height(), skeleton.tile_size());
        let read = walk(&self.doc, version, Keep::All, &mut NoSink, |parse, i| {
            if load.is_none() {
                head = parse.head_fields().to_vec();
                load = Some(CoreLoad::begin(&Fields::of(&head), version, size, budget)?);
            }
            if let Some(i) = i {
                load.as_mut()
                    .expect("頭で作った")
                    .layer(&Fields::of(parse.layer_fields()), i)?;
                parse.drop_layer_values();
            }
            Ok(())
        })?;
        load.ok_or_else(|| Error::InvalidData("正本の頭がありません".into()))?
            .finish(&Fields::of(&head), read.fields())
    }
    /// 全部をメモリの正本にする（小さな文書・試験のため。分けない正本で 512 MiB まで。超えれば読まずに断る）。
    pub fn to_native(&self) -> Result<NativeDocument> {
        check_budget(self.plan.total() <= MAX_ONE_ENTRY, TOO_BIG_FOR_MEMORY)?;
        let mut out = Vec::new();
        self.write_entry(Which::Full, &mut out)?;
        NativeDocument::read(&out)
    }
}
/// 作る正本のエントリ（`Made`）。
struct Entry {
    doc: Arc<CoreDoc>,
    which: Which,
}
impl Made for Entry {
    fn len(&self) -> u64 {
        self.doc.len_of(self.which)
    }
    fn sha256(&self) -> Result<String> {
        self.doc.sha_of(self.which)
    }
    fn write_to(&self, out: &mut dyn Write) -> Result<()> {
        self.doc.write_entry(self.which, out)
    }
    fn describe(&self) -> String {
        format!("{:?}", self.which)
    }
}
/// 1 つのエントリの中身だけを流す書き先。
struct EntrySink<'a> {
    out: &'a mut dyn Write,
    which: Which,
    ranges: &'a [(u64, u64)],
    plain_at: u64,
    value_at: u64,
    tile: Vec<u8>,
}
impl EntrySink<'_> {
    /// 値の流れの今の位置が、書くエントリに入るか。
    fn wants_value(&self, len: u64) -> bool {
        match self.which {
            Which::Full => true,
            Which::Header => false,
            Which::Part(k) => {
                let (s, e) = self.ranges[k];
                self.value_at >= s && self.value_at + len <= e
            }
        }
    }
}
impl Sink for EntrySink<'_> {
    fn plain(&mut self, b: &[u8]) -> Result<()> {
        let at = self.plain_at;
        self.plain_at += b.len() as u64;
        match self.which {
            Which::Full => self.out.write_all(b)?,
            // ヘッダーは識別子と版（はじめの 12 バイト）を書き換えたので、その後ろだけ
            Which::Header => {
                let skip = 12u64.saturating_sub(at).min(b.len() as u64) as usize;
                self.out.write_all(&b[skip..])?;
            }
            Which::Part(_) => {}
        }
        Ok(())
    }
    fn value(&mut self, b: &[u8]) -> Result<()> {
        if self.wants_value(b.len() as u64) {
            self.out.write_all(b)?;
        }
        self.value_at += b.len() as u64;
        Ok(())
    }
    fn tile(&mut self, surface: &Surface, coord: TileCoord) -> Result<()> {
        let len = surface.tile_bytes() as u64;
        if self.wants_value(len) {
            self.tile.resize(len as usize, 0);
            surface.copy_tile(coord, &mut self.tile)?;
            self.out.write_all(&self.tile)?;
        }
        self.value_at += len;
        Ok(())
    }
}
/// 1 回目の書き先: 分けない正本・ヘッダー・部分の SHA-256 を数える。
struct Hashes {
    full: Option<Sha256>,
    header: Option<Sha256>,
    parts: Vec<Sha256>,
    ranges: Ranges,
    plain_at: u64,
    value_at: u64,
}
impl Hashes {
    fn part_of(&self, len: u64) -> Option<usize> {
        // 区切りは値の境目にあるので、値の始まりで決まる
        let at = self.value_at;
        let k = self.ranges.partition_point(|&(_, e)| e <= at);
        (k < self.ranges.len() && at + len <= self.ranges[k].1).then_some(k)
    }
}
/// 書いた並びを受ける側（1 回目の SHA-256 など）。`walk` が並び（分けない形）を読み手へも渡す。
trait Observer {
    fn plain(&mut self, b: &[u8]);
    fn value(&mut self, b: &[u8]);
}
impl Observer for Hashes {
    fn plain(&mut self, b: &[u8]) {
        if let Some(h) = &mut self.full {
            h.update(b);
        }
        if let Some(h) = &mut self.header {
            let skip = 12u64.saturating_sub(self.plain_at).min(b.len() as u64) as usize;
            h.update(&b[skip..]);
        }
        self.plain_at += b.len() as u64;
    }
    fn value(&mut self, b: &[u8]) {
        if let Some(h) = &mut self.full {
            h.update(b);
        }
        if let Some(k) = self.part_of(b.len() as u64) {
            self.parts[k].update(b);
        }
        self.value_at += b.len() as u64;
    }
}
struct NoSink;
impl Observer for NoSink {
    fn plain(&mut self, _: &[u8]) {}
    fn value(&mut self, _: &[u8]) {}
}
/// 並びを層ごとに貯めて読み手へ渡す書き先。
struct Feed<'a> {
    buf: Rc<RefCell<Fed>>,
    observer: &'a mut dyn Observer,
}
#[derive(Default)]
struct Fed {
    bytes: Vec<u8>,
    at: usize,
}
impl Sink for Feed<'_> {
    fn plain(&mut self, b: &[u8]) -> Result<()> {
        self.observer.plain(b);
        self.buf.borrow_mut().bytes.extend_from_slice(b);
        Ok(())
    }
    fn value(&mut self, b: &[u8]) -> Result<()> {
        self.observer.value(b);
        self.buf.borrow_mut().bytes.extend_from_slice(b);
        Ok(())
    }
    fn tile(&mut self, surface: &Surface, coord: TileCoord) -> Result<()> {
        let mut fed = self.buf.borrow_mut();
        let at = fed.bytes.len();
        fed.bytes.resize(at + surface.tile_bytes(), 0);
        surface.copy_tile(coord, &mut fed.bytes[at..])?;
        self.observer.value(&fed.bytes[at..]);
        Ok(())
    }
}
/// 貯めた並びを読む側（読み手は、書き手が 1 つの層を書き終えてから、その層を読む）。
struct FedSource {
    buf: Rc<RefCell<Fed>>,
    out: Vec<u8>,
    at: u64,
}
impl ByteSource for FedSource {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let mut fed = self.buf.borrow_mut();
        let end = fed.at + n;
        if end > fed.bytes.len() {
            return Err(Error::InvalidData("正本が途中で切れています".into()));
        }
        self.out.clear();
        self.out.extend_from_slice(&fed.bytes[fed.at..end]);
        fed.at = end;
        if fed.at == fed.bytes.len() {
            fed.bytes.clear();
            fed.at = 0;
        }
        self.at += n as u64;
        Ok(&self.out)
    }
    fn position(&self) -> u64 {
        self.at
    }
    fn at_end(&mut self) -> Result<bool> {
        let fed = self.buf.borrow();
        Ok(fed.at >= fed.bytes.len())
    }
}
/// core の文書を正本の並び（分けない形）にしながら、層ごとに読み手で読む。`step` は頭を読んだあと（`None`）と各層を読んだあと
/// （`Some(層)`）に呼ばれる。持つのは層 1 枚ぶんの並び。読み手の骨組みか全部の項目を返す（`keep`）。
fn walk(
    doc: &Document,
    version: i32,
    keep: Keep,
    observer: &mut dyn Observer,
    mut step: impl FnMut(&mut Parse<'_>, Option<usize>) -> Result<()>,
) -> Result<NativeDocument> {
    let fed = Rc::new(RefCell::new(Fed::default()));
    let mut source = FedSource {
        buf: fed.clone(),
        out: Vec::new(),
        at: 0,
    };
    let mut feed = Feed { buf: fed, observer };
    // 層の前までを書いて読み、層は 1 つずつ書いて読む
    crate::core_bridge::write_head(&mut feed, doc, version)?;
    let mut parse = Parse::begin(&mut source, None, keep)?;
    step(&mut parse, None)?;
    for (i, layer) in doc.layers().iter().enumerate() {
        feed.layer(i)?;
        crate::core_bridge::write_layer_to(&mut feed, layer, version)?;
        let read = parse.next_layer()?;
        check(read == Some(i), "正本の層の数が合いません")?;
        step(&mut parse, Some(i))?;
    }
    check(parse.next_layer()?.is_none(), "正本の層の数が合いません")?;
    // 層の後（手動の ID の色）
    crate::core_bridge::write_tail(&mut feed, doc, version)?;
    parse.finish()
}
/// core へ渡せない項目があれば断る。
fn refuse_issues(skeleton: &NativeDocument) -> Result<()> {
    let issues = crate::core_bridge::core_issues_of(skeleton.fields());
    check(
        issues.is_empty(),
        format!("coreへの変換を拒否しました: {}", issues.join("、")),
    )
}

// ───────── メモリの正本を分ける ─────────

/// メモリの正本（`NativeDocument`）を書くエントリにする。分けない並び（中の版）が閾値を超えれば版 26 のヘッダーと部分にする。
pub(crate) fn native_entries(doc: &NativeDocument, prefix: &str) -> Vec<(String, Blob)> {
    let bytes = doc.to_bytes();
    let t = Thresholds::current();
    if (bytes.len() as u64) <= t.split_above {
        return vec![(format!("{prefix}document.utpaint"), Blob::from(bytes))];
    }
    drop(bytes);
    let (header, parts) = split_fields(doc.fields(), doc.version(), &t);
    std::iter::once((format!("{prefix}document.utpaint"), Blob::from(header)))
        .chain(
            parts
                .into_iter()
                .enumerate()
                .map(|(k, p)| (format!("{prefix}document.utpaint.{}", k + 1), Blob::from(p))),
        )
        .collect()
}
/// 項目の並びから、版 26 のヘッダーと部分を作る（`Bytes` の値は、はじめの識別子を除いて部分へ）。
pub(crate) fn split_fields(
    fields: &[NativeField],
    version: i32,
    t: &Thresholds,
) -> (Vec<u8>, Vec<Vec<u8>>) {
    let layer_of = |path: &str| -> Option<u32> {
        path.strip_prefix("layers[")
            .and_then(|r| r.split(']').next())
            .and_then(|n| n.parse().ok())
    };
    // 層より前の値は層 0、層より後（手動の ID の色の `tag`）の値は最後の層に数える（`PlanSink` が、書いた順の「今の層」に数えるのと同じ）
    let last_layer = fields
        .iter()
        .filter_map(|f| layer_of(&f.path))
        .max()
        .unwrap_or(0);
    let section = |path: &str| -> u32 {
        if path.starts_with("manual_id_colors.") {
            last_layer
        } else {
            layer_of(path).unwrap_or(0)
        }
    };
    let values: Vec<(u32, u32)> = fields
        .iter()
        .skip(1)
        .filter_map(|f| match &f.value {
            NativeValue::Bytes(b) => Some((section(&f.path), b.len() as u32)),
            _ => None,
        })
        .collect();
    let ranges = cut(&values, t);
    let mut header = Vec::new();
    header.extend_from_slice(b"DOTPAINT");
    header.extend(SPLIT_VERSION.to_le_bytes());
    header.extend(version.to_le_bytes());
    header.extend((ranges.len() as i32).to_le_bytes());
    let mut stream = Vec::new();
    // はじめの 2 つ（識別子と版）はヘッダーに書いた
    for f in fields.iter().skip(2) {
        match &f.value {
            NativeValue::Bytes(b) => stream.extend_from_slice(b),
            v => v.write(&mut header),
        }
    }
    let parts = ranges
        .iter()
        .map(|&(s, e)| stream[s as usize..e as usize].to_vec())
        .collect();
    (header, parts)
}

// ───────── 置いてある正本（.ylp のエントリ・復旧の中身）を流して読む ─────────

/// 置いてある正本: 分けない正本ならヘッダーだけ、版 26 ならヘッダーと部分（番号の順）。
#[derive(Clone, Debug)]
pub(crate) struct StoredDoc {
    pub header: Blob,
    pub parts: Vec<Blob>,
}
impl StoredDoc {
    fn part_stream(&self, content: bool) -> Result<Option<PartStream>> {
        if self.parts.is_empty() {
            return Ok(None);
        }
        let parts = self
            .parts
            .iter()
            .map(|p| {
                Ok(if content {
                    Part::Blob(p.clone())
                } else {
                    Part::Length(p.len())
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(PartStream::new(parts)?))
    }
    /// 骨組みを読む（全部を流して確かめる。`content` が false なら版 26 の部分は長さだけ見て中身を読まない）。
    pub fn skeleton(&self, content: bool) -> Result<NativeDocument> {
        let mut src = StreamSource::new(self.header.reader()?);
        let mut parts = self.part_stream(content)?;
        let mut parse = Parse::begin(&mut src, parts.as_mut(), Keep::Skeleton)?;
        while parse.next_layer()?.is_some() {}
        parse.finish()
    }
    /// 流して core の文書にする（持つのは層 1 枚ぶん）。
    pub fn to_core(&self, skeleton: &NativeDocument, budget: Option<u64>) -> Result<Document> {
        refuse_issues(skeleton)?;
        let mut src = StreamSource::new(self.header.reader()?);
        let mut parts = self.part_stream(true)?;
        let mut parse = Parse::begin(&mut src, parts.as_mut(), Keep::All)?;
        let head = parse.head_fields().to_vec();
        let version = parse.version;
        let size = (parse.width, parse.height, parse.tile_size);
        let mut load = CoreLoad::begin(&Fields::of(&head), version, size, budget)?;
        while let Some(i) = parse.next_layer()? {
            load.layer(&Fields::of(parse.layer_fields()), i)?;
            parse.drop_layer_values();
        }
        let read = parse.finish()?;
        load.finish(&Fields::of(&head), read.fields())
    }
    /// 全部をメモリの正本にする（小さな文書・試験。ヘッダーと部分の合計で 512 MiB まで。超えれば読まずに断る）。
    pub fn to_native(&self) -> Result<NativeDocument> {
        let total = self
            .parts
            .iter()
            .fold(self.header.len(), |sum, p| sum.saturating_add(p.len()));
        check_budget(total <= MAX_ONE_ENTRY, TOO_BIG_FOR_MEMORY)?;
        let header = self.header.bytes()?;
        if self.parts.is_empty() {
            return NativeDocument::read(&header);
        }
        let parts = self
            .parts
            .iter()
            .map(|p| p.bytes())
            .collect::<Result<Vec<_>>>()?;
        let refs: Vec<&[u8]> = parts.iter().map(|p| &p[..]).collect();
        NativeDocument::read_split(&header, &refs)
    }
}

// ───────── セットの正本 ─────────

/// 書くセットの正本の元: メモリの正本か、core の文書（保存・書き置きの写し。画素は流して正本にする）。
#[derive(Clone)]
pub enum DocumentSource {
    Native(NativeDocument),
    Core(Arc<Document>),
}
impl std::fmt::Debug for DocumentSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native(d) => write!(f, "Native({} 層)", d.layer_count()),
            Self::Core(d) => write!(f, "Core({} 層)", d.layers().len()),
        }
    }
}
impl DocumentSource {
    /// core の文書（保存・書き置きのための写し）から。正本に書けるかを先に確かめる（進行中のストローク・寸法など。
    /// `NativeDocument::from_core` と同じ断り）。画素は写さない。
    pub fn from_core(doc: Arc<Document>) -> Result<Self> {
        crate::core_bridge::check_writable(&doc)?;
        Ok(Self::Core(doc))
    }
}
impl From<NativeDocument> for DocumentSource {
    fn from(d: NativeDocument) -> Self {
        Self::Native(d)
    }
}
impl From<Arc<Document>> for DocumentSource {
    fn from(d: Arc<Document>) -> Self {
        Self::Core(d)
    }
}
impl From<Document> for DocumentSource {
    fn from(d: Document) -> Self {
        Self::Core(Arc::new(d))
    }
}

/// テクスチャセットの正本。置いてある正本（.ylp のエントリ・復旧の中身。分けない形か版 26）、メモリの正本、core の文書から作る正本の
/// どれか。画素は持たず（置いてある正本）、要るときに流して読む。頭の値（ID・寸法・版・層の数）はいつでも安く読める。
#[derive(Clone)]
pub struct SetDocument(Arc<SetDoc>);
struct SetDoc {
    /// .ylp に置くエントリ（ヘッダーと部分。変わっていないかを中身を読まずに見分ける）。
    entries: Vec<Blob>,
    origin: Origin,
    head: Head,
    /// 骨組み（`Bytes` の値を持たない項目）。core の文書から作る正本は、初めて要るときに流して作る。
    skeleton: OnceLock<std::result::Result<Arc<NativeDocument>, String>>,
}
enum Origin {
    Stored(StoredDoc),
    Core(Arc<CoreDoc>),
    Native(Arc<NativeDocument>),
}
#[derive(Clone, Debug)]
struct Head {
    id: String,
    width: i32,
    height: i32,
    tile_size: i32,
    version: i32,
    layers: usize,
    split: bool,
}
impl Head {
    fn of(doc: &NativeDocument, split: bool) -> Self {
        Self {
            id: doc.id().to_owned(),
            width: doc.width(),
            height: doc.height(),
            tile_size: doc.tile_size(),
            version: doc.version(),
            layers: doc.layer_count(),
            split,
        }
    }
}
impl std::fmt::Debug for SetDocument {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetDocument")
            .field("head", &self.0.head)
            .field("entries", &self.0.entries)
            .finish()
    }
}
impl SetDocument {
    /// メモリの正本から（どこにも置いていない正本。試験や、メモリの正本を `SetDocument` として扱うため）。
    pub fn in_memory(doc: NativeDocument) -> Self {
        let source = DocumentSource::Native(doc);
        Self::from_source(&source, "")
            .expect("メモリの正本は作れる")
            .0
    }
    /// 置いてある正本（ヘッダーと番号の順の部分）。骨組みを読んで確かめる（版 26 の部分は長さと数だけ。中身は読むときに確かめる）。
    pub(crate) fn stored(header: Blob, parts: Vec<Blob>) -> Result<Self> {
        let stored = StoredDoc { header, parts };
        let skeleton = stored.skeleton(stored.parts.is_empty())?;
        Ok(Self::stored_with(stored, Arc::new(skeleton)))
    }
    /// 置いてある正本と、読んで確かめ済みの骨組み。
    pub(crate) fn stored_with(stored: StoredDoc, skeleton: Arc<NativeDocument>) -> Self {
        let head = Head::of(&skeleton, !stored.parts.is_empty());
        let entries = std::iter::once(stored.header.clone())
            .chain(stored.parts.iter().cloned())
            .collect();
        Self(Arc::new(SetDoc {
            entries,
            origin: Origin::Stored(stored),
            head,
            skeleton: OnceLock::from(Ok(skeleton)),
        }))
    }
    /// 元から作る正本と、`prefix`（`sets/<ID>/` か根なら空）の下に置くエントリ。
    pub(crate) fn from_source(
        source: &DocumentSource,
        prefix: &str,
    ) -> Result<(Self, Vec<(String, Blob)>)> {
        match source {
            DocumentSource::Native(doc) => {
                let entries = native_entries(doc, prefix);
                let me = Self(Arc::new(SetDoc {
                    entries: entries.iter().map(|(_, b)| b.clone()).collect(),
                    head: Head::of(doc, entries.len() > 1),
                    origin: Origin::Native(Arc::new(doc.clone())),
                    skeleton: OnceLock::new(),
                }));
                Ok((me, entries))
            }
            DocumentSource::Core(doc) => {
                let made = CoreDoc::new(doc.clone())?;
                let entries = made.entries(prefix);
                let head = Head {
                    id: crate::guid(&crate::core_bridge::native_id(doc.id())),
                    width: doc.width() as i32,
                    height: doc.height() as i32,
                    tile_size: doc.tile_size() as i32,
                    version: made.plan().version,
                    layers: doc.layers().len(),
                    split: made.plan().split.is_some(),
                };
                let me = Self(Arc::new(SetDoc {
                    entries: entries.iter().map(|(_, b)| b.clone()).collect(),
                    head,
                    origin: Origin::Core(made),
                    skeleton: OnceLock::new(),
                }));
                Ok((me, entries))
            }
        }
    }
    /// 同じ正本か（複製したものどうし。試験が、骨組みを読み直さずに同じ正本を使ったかを見るため）。
    #[cfg(test)]
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    /// エントリ（ヘッダーと部分）が、この正本の作ったものと同じか（変わっていなければ骨組みを読み直さない）。
    pub(crate) fn placed_as(&self, entries: &[&Blob]) -> bool {
        self.0.entries.len() == entries.len()
            && self.0.entries.iter().zip(entries).all(|(a, b)| a.same(b))
    }
    /// 文書の ID（GUID の文字列。`NativeDocument::id` と同じ形）。
    pub fn id(&self) -> &str {
        &self.0.head.id
    }
    pub fn width(&self) -> i32 {
        self.0.head.width
    }
    pub fn height(&self) -> i32 {
        self.0.head.height
    }
    pub fn tile_size(&self) -> i32 {
        self.0.head.tile_size
    }
    /// 中身の正本の版（版 26 で分けていても、中の版）。
    pub fn version(&self) -> i32 {
        self.0.head.version
    }
    pub fn layer_count(&self) -> usize {
        self.0.head.layers
    }
    /// 版 26（ヘッダーと部分）で置く・置いてあるか。
    pub fn is_split(&self) -> bool {
        self.0.head.split
    }
    /// 骨組み（`Bytes` の値（色・画素）を持たない項目。層の構造・名前・ID・効果の設定）。core の文書から作る正本は、初めて呼ぶと
    /// 流して作る。
    pub fn skeleton(&self) -> Result<Arc<NativeDocument>> {
        self.0
            .skeleton
            .get_or_init(|| {
                match &self.0.origin {
                    Origin::Stored(s) => s.skeleton(true).map(Arc::new),
                    Origin::Core(c) => c.skeleton(),
                    Origin::Native(n) => Ok(n.clone()),
                }
                .map_err(|e| e.to_string())
            })
            .clone()
            .map_err(Error::InvalidData)
    }
    /// core へ渡すと失われる項目（`NativeDocument::core_issues` と同じ）。core の文書から作った正本には無い。
    pub fn core_issues(&self) -> Vec<String> {
        match &self.0.origin {
            Origin::Core(_) => Vec::new(),
            Origin::Native(n) => n.core_issues(),
            Origin::Stored(_) => match self.skeleton() {
                Ok(s) => crate::core_bridge::core_issues_of(s.fields()),
                Err(e) => vec![e.to_string()],
            },
        }
    }
    /// 編集用の core の文書にする（`NativeDocument::to_core` と同じ意味）。置いてある正本は流して読み、持つのは層 1 枚ぶん。
    pub fn to_core(&self) -> Result<Document> {
        self.to_core_within(None)
    }
    /// `to_core` の画素の予算を指定できる形（超えたら途中で止めて `Error::Budget`）。
    pub fn to_core_within(&self, source_budget: Option<u64>) -> Result<Document> {
        match &self.0.origin {
            Origin::Stored(s) => s.to_core(&*self.skeleton()?, source_budget),
            Origin::Core(c) => c.to_core(source_budget),
            Origin::Native(n) => n.to_core_within(source_budget),
        }
    }
    /// 全部をメモリの正本にする（小さな文書・試験。512 MiB まで。超えれば読まずに `Error::Budget`）。
    pub fn to_native(&self) -> Result<NativeDocument> {
        match &self.0.origin {
            Origin::Stored(s) => s.to_native(),
            Origin::Core(c) => c.to_native(),
            Origin::Native(n) => Ok((**n).clone()),
        }
    }
    /// 分けない形（中の版）のバイト列（小さな文書・試験。512 MiB まで）。
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        Ok(self.to_native()?.to_bytes())
    }
    /// 値を 1 つ差し替えたメモリの正本（`NativeDocument::with_value`。小さな文書のため）。
    pub fn with_value(&self, path: &str, value: NativeValue) -> Result<NativeDocument> {
        self.to_native()?.with_value(path, value)
    }
    /// ファイルの位置で持つエントリの置き場の名前を、保存で置き換えた後の名前へ付け替える（`Blob::note_path`）。
    pub(crate) fn note_path(&self, path: &std::path::Path) {
        if let Origin::Stored(s) = &self.0.origin {
            s.header.note_path(path);
            for p in &s.parts {
                p.note_path(path);
            }
        }
    }
    /// 同じ中身（エントリごとの長さと SHA-256）を、別の置き場のエントリ（保存で置き換えた後のファイル）から読む正本にする。中身が
    /// 違えば None。骨組みは読み直さない。
    pub(crate) fn repointed(&self, header: &Blob, parts: &[Blob]) -> Option<Self> {
        let placed: Vec<&Blob> = std::iter::once(header).chain(parts).collect();
        if placed.len() != self.0.entries.len() {
            return None;
        }
        for (old, new) in self.0.entries.iter().zip(&placed) {
            if old.len() != new.len() || old.sha256().ok()? != new.sha256().ok()? {
                return None;
            }
        }
        let skeleton = self.skeleton().ok()?;
        Some(Self::stored_with(
            StoredDoc {
                header: header.clone(),
                parts: parts.to_vec(),
            },
            skeleton,
        ))
    }
}

#[cfg(test)]
#[path = "bigdoc_tests.rs"]
mod tests;
