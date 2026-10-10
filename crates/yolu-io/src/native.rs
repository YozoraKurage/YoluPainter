use crate::{check, check_budget, guid, is_hash, Error, Result, MAX_ENTRY_BYTES};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// Unity 版（0.2.0 の `DocumentBinary`）が書き、読める正本の一番新しい版。
pub const UNITY_NATIVE_VERSION: i32 = 21;
/// 文書のユーザーチャンネル（core の 6〜63）の一覧を足した版。ユーザーチャンネルのある文書だけがこの版になり、
/// Unity 版の読み手は「Unsupported archive version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const USER_CHANNELS_VERSION: i32 = 22;
/// Rust 版だけの Generator の種類（ノイズ 64・グランジ 65）を足した版。版 22 の中身（ユーザーチャンネルの一覧。この版では 0 個も書く）に、
/// Generator の種類 64・65 とその欄が加わる。これを使う文書だけがこの版になり、Unity 版の読み手は「Unsupported archive version」で断る
/// （形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const PROCEDURAL_VERSION: i32 = 23;
/// Rust 版だけの色調補正（調整レイヤーとフィルターの段の種類 64〜69: グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・
/// 2 値化・ポスタリゼーション）を足した版。版 23 の中身に、調整・フィルターの種類 64〜69 とその欄（`color_adjust` の並び）が加わる。これを使う
/// 文書だけがこの版になり、Unity 版の読み手は「Unsupported archive version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const ADJUST_VERSION: i32 = 24;
/// グラデーションマップの混色（混色モード・輝度の補正）と区間ごとの混合率曲線を足した版。版 24 の中身に、種類 64（グラデーションマップ）の欄の
/// ランプのあとへ混色の欄が加わる。これらを使うグラデーションマップのある文書だけがこの版になり、Unity 版の読み手は「Unsupported archive
/// version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const MIXING_VERSION: i32 = 25;
/// レイヤーのパスの一覧（1 つのレイヤーに何本ものパス、パスごとの名前・表示）を足した版。版 25 の中身に、レイヤーの属性のビット 6 とパスの一覧の塊
/// （`paths`）が加わる。一覧を使うレイヤー（塗りつぶしレイヤーのパス・2 本以上のパス・名前や隠すパス・ストローク／消しゴム以外の種類・筆先・
/// 角度・深さ・対称の設定・角や取っ手の点を持つパス）のある文書だけがこの版になり、Unity 版の読み手は「Unsupported archive version」で
/// 断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const PATHS_VERSION: i32 = 27;
/// 0.5.0 の新しい効果を足した版。版 25 の中身に、フィルターの段の種類 70〜79（ヒストグラムスキャン・ヒストグラムレンジ・スロープぼかし・方向のぼかし・
/// ゆがみ・モルフォロジー・エッジ検出・ハイパス・メディアン・グロー）と、Generator の種類 66（模様）・67（アイランドごとのばらつき）・68（ライト）・69（マスクの組み立て）・70（画像）が加わる
/// （種類ごとの欄は `effect` の塊）。これを使う文書だけがこの版になり、版 25 までの読み手（スタンドアロン 0.4.x）は版の範囲の外として、Unity 版は
/// 「Unsupported archive version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const EFFECTS_VERSION: i32 = 28;
/// 塗りつぶしの点のグラデーション（レイヤーの属性のビット 7 の続きの属性の印 `attributes_ext` のビット 0）と、塗りつぶしの画像ごとの異方性のフィルターの入・切（`images[i].anisotropic`）を
/// 足した版。点のグラデーションか、異方性を切った画像のある文書だけがこの版になり、それより古い読み手は版の範囲の外として断る
/// （形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const POINT_GRADIENT_VERSION: i32 = 29;
/// レイヤーのフィルターが UV の継ぎ目をまたぐかの文書の設定（頭の `filter_seams`）を足した版。設定を切った（既定の入から変えた）文書だけが
/// この版になり、0.4.x のスタンドアロンは版の範囲の外、Unity 版の読み手は「Unsupported archive version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
/// この版の文書は版 27〜30 の中身も読み書きできる。
pub const SEAMS_VERSION: i32 = 32;
/// テキストレイヤー（ラスターレイヤーの文字の値。続きの属性の印のビット 1 と `text` の塊）を足した版。テキストレイヤーのある文書だけがこの版になり
/// （版 27〜29 の中身も読み書きできる）、版 25 までの読み手（スタンドアロン 0.4.x）は版の範囲の外として、Unity 版は「Unsupported archive version」で
/// 断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
pub const TEXT_VERSION: i32 = 30;
/// 重なった UV のテクセルの持ち主の決め方（ベイクの優先。頭の `bake_priority`）を足した版。決め方を既定（番号の小さい三角形・外さない・
/// 手で選んだアイランドなし）から変えた文書だけがこの版になり、0.4.x のスタンドアロンは版の範囲の外、Unity 版の読み手は「Unsupported archive
/// version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。この版の文書は版 27〜32 の中身も読み書きできる。
pub const BAKE_PRIORITY_VERSION: i32 = 33;
/// パスのブラシのアンチエイリアスの段（パスの `brush` の塊の `anti_alias`）を加えた版。段が なし でないパスのある文書だけがこの版になり、
/// 0.5.x までのスタンドアロンは版の範囲の外、Unity 版の読み手は「Unsupported archive version」で断る（形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。
/// この版の文書は版 27〜33 の中身も読み書きできる。
pub const ANTI_ALIAS_VERSION: i32 = 34;
/// 定規（レイヤー・グループに付く、描くときの寄せ先と対称。レイヤーの続きの属性の印のビット 2 と、レイヤーの末尾の `ruler_count`・`rulers[i]`）と、
/// パスの 2D の対称の種類 5（線対称・角度つき。`canvas_symmetry` の `mode` が 5 のとき `angle` が続く）を足した版。定規のあるレイヤーか、線対称の 2D のパスの
/// ある文書だけがこの版になり、0.5.x までのスタンドアロンは版の範囲の外、Unity 版の読み手は「Unsupported archive version」で断る
/// （形式は docs/YLP_FORMAT.md、理由は docs/YLP_DECISIONS.md）。この版の文書は版 27〜34 の中身も読み書きできる。
pub const RULERS_VERSION: i32 = 35;
/// この読み手が読める一番新しい版。読める版の集合は 1〜`MIXING_VERSION`・`SPLIT_VERSION`（26。分けた正本の識別）・`PATHS_VERSION`（27）・
/// `EFFECTS_VERSION`（28）・`POINT_GRADIENT_VERSION`（29）・`TEXT_VERSION`（30）・`SEAMS_VERSION`（32）・`BAKE_PRIORITY_VERSION`（33）・
/// `ANTI_ALIAS_VERSION`（34）・`RULERS_VERSION`（35）で、間の 31 は意味を決めておらず断る（版を割り振ったら `is_known_version` へ足す）。
pub const MAX_NATIVE_VERSION: i32 = RULERS_VERSION;
/// レイヤーの後に手動の ID の色の塊（`YLID`）を置ける版。書き手の版（21 以上）はどれもこれ以上なので、色のために版を上げることは無い。
pub(crate) const MANUAL_ID_COLORS_VERSION: i32 = 19;
/// 標準のチャンネルの数（番号 0〜5。Unity 版の PaintChannel）。
const STANDARD_CHANNELS: i32 = 6;
/// 版 22 のユーザーチャンネル（番号 → 種類: 0 色・1 スカラー・2 法線）。版 21 までは空。
type UserChannels = BTreeMap<i32, i32>;

/// 正本の値。f64 は演算せずビットを保存し、RGBA・GUID の並びも変えない。
#[derive(Clone, Debug, PartialEq)]
pub enum NativeValue {
    Int(i32),
    Byte(u8),
    Bool(bool),
    Float(f64),
    Guid([u8; 16]),
    Text(String),
    Bytes(Arc<[u8]>),
}
impl NativeValue {
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(v) => out.extend(v.to_le_bytes()),
            Self::Byte(v) => out.push(*v),
            Self::Bool(v) => out.push(u8::from(*v)),
            Self::Float(v) => out.extend(v.to_le_bytes()),
            Self::Guid(v) => out.extend(v),
            Self::Text(v) => {
                out.extend((v.len() as i32).to_le_bytes());
                out.extend(v.as_bytes());
            }
            Self::Bytes(v) => out.extend(v.as_ref()),
        }
    }
}
/// `layers[0].channels[0].tiles[0].rgba` などのパスでアクセスする、ディスク順の値。
#[derive(Clone, Debug, PartialEq)]
pub struct NativeField {
    pub path: String,
    pub value: NativeValue,
}
#[derive(Clone, Debug)]
pub struct NativeDocument {
    version: i32,
    id: String,
    width: i32,
    height: i32,
    tile_size: i32,
    layers: usize,
    fields: Vec<NativeField>,
    /// `Bytes` の値（画素・色）を持たない骨組みか（[`Keep::Skeleton`]）。
    skeleton: bool,
}
impl NativeDocument {
    pub fn version(&self) -> i32 {
        self.version
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
    pub fn tile_size(&self) -> i32 {
        self.tile_size
    }
    pub fn layer_count(&self) -> usize {
        self.layers
    }
    pub fn fields(&self) -> &[NativeField] {
        &self.fields
    }
    pub fn field(&self, path: &str) -> Option<&NativeValue> {
        self.fields
            .iter()
            .find(|f| f.path == path)
            .map(|f| &f.value)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        debug_assert!(
            !self.skeleton,
            "骨組み（Bytes の値の無い項目）は正本として書けません"
        );
        let mut out = Vec::new();
        for f in &self.fields {
            f.value.write(&mut out);
        }
        out
    }
    /// 値を差し替えて全体を再検証する。未知の構造や不整合を保存に持ち越さない。
    pub fn with_value(&self, path: &str, value: NativeValue) -> Result<Self> {
        self.with_values(&[(path.to_owned(), value)])
    }
    /// 値をまとめて差し替えて、全体を 1 回だけ再検証する（`with_value` と同じ決まり。同じ項目を 2 回は差し替えない）。
    pub fn with_values(&self, changes: &[(String, NativeValue)]) -> Result<Self> {
        check(!self.skeleton, "骨組みの正本は書き換えられません")?;
        let mut fields = self.fields.clone();
        for (path, value) in changes {
            let field = fields
                .iter_mut()
                .find(|f| &f.path == path)
                .ok_or_else(|| Error::InvalidData(format!("正本の項目がありません: {path}")))?;
            check(
                std::mem::discriminant(&field.value) == std::mem::discriminant(value),
                "正本の値の型を変更できません",
            )?;
            field.value = value.clone();
        }
        let mut out = Vec::new();
        for f in fields {
            f.value.write(&mut out);
        }
        Self::read(&out)
    }
    pub fn read(b: &[u8]) -> Result<Self> {
        check_budget(
            b.len() <= MAX_ENTRY_BYTES,
            "正本の512 MiB予算を超えています",
        )?;
        let mut src = SliceSource { bytes: b, at: 0 };
        let mut parse = Parse::begin(&mut src, None, Keep::All)?;
        while parse.next_layer()?.is_some() {}
        parse.finish()
    }
    /// 版 26（分けた正本）の読み: ヘッダー（`document.utpaint`）と部分（`document.utpaint.1`…の順）。項目は分けていない正本
    /// （中の版）を読んだのと同じになる。
    pub(crate) fn read_split(header: &[u8], parts: &[&[u8]]) -> Result<Self> {
        let mut src = SliceSource {
            bytes: header,
            at: 0,
        };
        let mut stream = PartStream::new(
            parts
                .iter()
                .map(|p| Part::Reader(Box::new(std::io::Cursor::new(p.to_vec())), p.len() as u64))
                .collect(),
        )?;
        let mut parse = Parse::begin(&mut src, Some(&mut stream), Keep::All)?;
        while parse.next_layer()?.is_some() {}
        parse.finish()
    }
    /// 骨組み（`Bytes` の値を持たない項目。レイヤーの構造・名前・ID・効果の設定）だけを持つか。骨組みは `to_bytes`・`to_core` に使わない。
    pub(crate) fn is_skeleton(&self) -> bool {
        self.skeleton
    }
}
/// 正本の項目の残し方。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keep {
    /// 全部（画素も）。
    All,
    /// `Bytes` の値（画素・色）を残さない骨組み。値は読んで確かめる（部分の中身が無い読みでは長さだけ）。
    Skeleton,
}
/// 正本の並びの読み元（メモリのバイト列か、流れ）。1 回に取るのは 1 つの値（タイルなら最大 1 MiB）。
pub(crate) trait ByteSource {
    /// 次の `n` バイト。足りなければ、どこで切れたかを添えて断る。
    fn take(&mut self, n: usize) -> Result<&[u8]>;
    /// 読んだバイト数。
    fn position(&self) -> u64;
    /// もう読むものが無いか。
    fn at_end(&mut self) -> Result<bool>;
    /// 中身が読めるか（長さだけの読みは false。中身の確かめを飛ばす）。
    fn has_content(&self) -> bool {
        true
    }
}
pub(crate) struct SliceSource<'a> {
    pub bytes: &'a [u8],
    pub at: usize,
}
impl ByteSource for SliceSource<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
        let s = self
            .bytes
            .get(self.at..end)
            .ok_or_else(|| Error::InvalidData("正本が途中で切れています".into()))?;
        self.at = end;
        Ok(s)
    }
    fn position(&self) -> u64 {
        self.at as u64
    }
    fn at_end(&mut self) -> Result<bool> {
        Ok(self.at >= self.bytes.len())
    }
}
/// 流れ（ファイルのエントリの展開など）から読む。読みの失敗（壊れた中身・SHA-256 の不一致）は `InvalidData` にする。
pub(crate) struct StreamSource<R: std::io::Read> {
    r: R,
    buf: Vec<u8>,
    at: u64,
    peeked: Option<u8>,
}
impl<R: std::io::Read> StreamSource<R> {
    pub fn new(r: R) -> Self {
        Self {
            r,
            buf: Vec::new(),
            at: 0,
            peeked: None,
        }
    }
    fn fill(&mut self, out_from: usize) -> Result<()> {
        let mut at = out_from;
        while at < self.buf.len() {
            match self.r.read(&mut self.buf[at..]) {
                Ok(0) => return Err(Error::InvalidData("正本が途中で切れています".into())),
                Ok(n) => at += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(crate::package::io_error(e)),
            }
        }
        Ok(())
    }
}
impl<R: std::io::Read> ByteSource for StreamSource<R> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        // 値は大きくてもタイル 1 枚（1 MiB）なので、宣言の長さの確保は正本の値の上限で抑える
        check_budget(n <= 1 << 20, "正本の値が大きすぎます")?;
        self.buf.clear();
        self.buf.resize(n, 0);
        let mut from = 0;
        if n > 0 {
            if let Some(b) = self.peeked.take() {
                self.buf[0] = b;
                from = 1;
            }
        }
        self.fill(from)?;
        self.at += n as u64;
        Ok(&self.buf)
    }
    fn position(&self) -> u64 {
        self.at
    }
    fn at_end(&mut self) -> Result<bool> {
        if self.peeked.is_some() {
            return Ok(false);
        }
        let mut one = [0u8; 1];
        loop {
            match self.r.read(&mut one) {
                Ok(0) => return Ok(true),
                Ok(_) => {
                    self.peeked = Some(one[0]);
                    return Ok(false);
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err(crate::package::io_error(e)),
            }
        }
    }
}
/// 版 26 の部分の 1 つ（流れと長さ、流れを作るエントリ（読む番が来てから開く）、または長さだけ）。
pub(crate) enum Part {
    Reader(Box<dyn std::io::Read + Send>, u64),
    Blob(crate::Blob),
    Length(u64),
}
impl Part {
    fn len(&self) -> u64 {
        match self {
            Self::Reader(_, n) | Self::Length(n) => *n,
            Self::Blob(b) => b.len(),
        }
    }
}
/// 版 26 の部分を順につないだ `Bytes` の値の流れ。値は部分の境目をまたげず、空の部分・余り・足りない部分は断る。
pub(crate) struct PartStream {
    parts: std::collections::VecDeque<Part>,
    current: Option<Part>,
    remaining: u64,
    total: usize,
    buf: Vec<u8>,
}
impl PartStream {
    pub fn new(parts: Vec<Part>) -> Result<Self> {
        check(!parts.is_empty(), "正本の部分がありません")?;
        for p in &parts {
            check(p.len() > 0, "正本の部分が空です")?;
        }
        Ok(Self {
            total: parts.len(),
            parts: parts.into(),
            current: None,
            remaining: 0,
            buf: Vec::new(),
        })
    }
    pub fn count(&self) -> usize {
        self.total
    }
    /// 今の部分を読み終えたことを確かめる（流れなら終わりまで読み、長さと SHA-256 の確かめを起こす）。
    fn close_current(&mut self) -> Result<()> {
        if let Some(Part::Reader(mut r, _)) = self.current.take() {
            let mut one = [0u8; 1];
            loop {
                match r.read(&mut one) {
                    Ok(0) => break,
                    Ok(_) => return Err(Error::InvalidData("正本の部分に余りがあります".into())),
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(crate::package::io_error(e)),
                }
            }
        }
        Ok(())
    }
    /// 全部の部分を読み終えたか（余りも足りない部分も無いか）。
    pub fn finish(&mut self) -> Result<()> {
        check(self.remaining == 0, "正本の部分に余りがあります")?;
        self.close_current()?;
        check(self.parts.is_empty(), "正本の部分が余っています")
    }
}
impl ByteSource for PartStream {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        check_budget(n <= 1 << 20, "正本の値が大きすぎます")?;
        if self.remaining == 0 && n > 0 {
            self.close_current()?;
            let next = self
                .parts
                .pop_front()
                .ok_or_else(|| Error::InvalidData("正本の部分が足りません".into()))?;
            self.remaining = next.len();
            // エントリは読む番が来てから開く（部分の数だけファイルを開いたままにしない）
            self.current = Some(match next {
                Part::Blob(b) => Part::Reader(b.reader()?, b.len()),
                other => other,
            });
        }
        check(
            n as u64 <= self.remaining,
            "正本の値が部分の境目をまたいでいます",
        )?;
        self.remaining -= n as u64;
        self.buf.clear();
        self.buf.resize(n, 0);
        if let Some(Part::Reader(r, _)) = &mut self.current {
            let mut at = 0;
            while at < n {
                match r.read(&mut self.buf[at..]) {
                    Ok(0) => {
                        return Err(Error::InvalidData("正本の部分が途中で切れています".into()))
                    }
                    Ok(k) => at += k,
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => return Err(crate::package::io_error(e)),
                }
            }
        }
        Ok(&self.buf)
    }
    fn position(&self) -> u64 {
        0
    }
    fn at_end(&mut self) -> Result<bool> {
        Ok(self.remaining == 0 && self.parts.is_empty())
    }
    fn has_content(&self) -> bool {
        !matches!(self.current, Some(Part::Length(_)))
            && !matches!(self.parts.front(), Some(Part::Length(_)))
    }
}

/// 版 26（分けた正本）の識別の版。ヘッダーは `DOTPAINT`・26・中の版・部分の数・中の版の並びから `Bytes` の値を抜いたもの。
pub const SPLIT_VERSION: i32 = 26;

/// 正本の版の数（外の版）が、意味の決まった版か。1〜`MIXING_VERSION`・`SPLIT_VERSION`・`PATHS_VERSION`・`EFFECTS_VERSION`・
/// `POINT_GRADIENT_VERSION`・`TEXT_VERSION`・`SEAMS_VERSION`・`BAKE_PRIORITY_VERSION`・`ANTI_ALIAS_VERSION`・`RULERS_VERSION` だけで、間の 31 は読まない（`MAX_NATIVE_VERSION` までの範囲で通すと、意味の無い版を版 25 の並びとして読んでしまう）。
fn is_known_version(version: i32) -> bool {
    (1..=MIXING_VERSION).contains(&version)
        || version == SPLIT_VERSION
        || version == PATHS_VERSION
        || version == EFFECTS_VERSION
        || version == POINT_GRADIENT_VERSION
        || version == SEAMS_VERSION
        || version == TEXT_VERSION
        || version == BAKE_PRIORITY_VERSION
        || version == ANTI_ALIAS_VERSION
        || version == RULERS_VERSION
}

/// 正本をレイヤーごとに読む（頭 → レイヤー 0, 1, … → 終わり）。レイヤーごとに項目を取り出せる（流して core へ入れる読みが、レイヤー 1 枚ぶんだけ持つため）。
pub(crate) struct Parse<'a> {
    r: Reader<'a>,
    pub version: i32,
    pub width: i32,
    pub height: i32,
    pub tile_size: i32,
    id: String,
    user: UserChannels,
    count: i32,
    next: i32,
    layers: Vec<Layer>,
    ids: HashSet<[u8; 16]>,
    anchor_ids: HashMap<[u8; 16], i32>,
    /// 文書の中の定規の ID（重ならない）。
    ruler_ids: HashSet<[u8; 16]>,
    /// 今のレイヤーの項目の始まり（`fields` の位置）。
    layer_start: usize,
}
impl<'a> Parse<'a> {
    /// 識別子からレイヤーの数までを読む。版 26 なら `parts` が要る（部分の数がヘッダーと合うこと）、ほかの版なら要らない。
    pub fn begin(
        src: &'a mut dyn ByteSource,
        parts: Option<&'a mut PartStream>,
        keep: Keep,
    ) -> Result<Self> {
        let mut r = Reader {
            src,
            parts: None,
            prefix: String::new(),
            fields: Vec::new(),
            keep,
        };
        let magic = r.take(8)? == b"DOTPAINT";
        check(magic, "正本の識別子が不正です")?;
        r.add("magic", NativeValue::Bytes(Arc::from(&b"DOTPAINT"[..])));
        let stored = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
        let version = if stored == SPLIT_VERSION {
            let inner = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
            // 中の版は分けていない並びの版（26 は分けた正本の印で、中には入らない）
            check(
                inner >= UNITY_NATIVE_VERSION && is_known_version(inner) && inner != SPLIT_VERSION,
                format!("分けた正本の中の版 {inner} は未対応です"),
            )?;
            let count = i32::from_le_bytes(r.take(4)?.try_into().unwrap());
            let parts =
                parts.ok_or_else(|| Error::InvalidData("分けた正本の部分がありません".into()))?;
            check(
                count >= 1 && count as usize == parts.count(),
                "分けた正本の部分の数が一致しません",
            )?;
            r.parts = Some(parts);
            inner
        } else {
            check(
                is_known_version(stored),
                format!(
                    ".version の値 {stored} は未対応または範囲外です (1..={MIXING_VERSION}・{SPLIT_VERSION}・{PATHS_VERSION}・{EFFECTS_VERSION}・{POINT_GRADIENT_VERSION}・{TEXT_VERSION}・{SEAMS_VERSION}・{BAKE_PRIORITY_VERSION}・{ANTI_ALIAS_VERSION}・{RULERS_VERSION})"
                ),
            )?;
            check(parts.is_none(), "分けていない正本に部分があります")?;
            stored
        };
        r.add("version", NativeValue::Int(version));
        let id = guid(&r.id("id", false)?);
        let edge = crate::MAX_DOCUMENT_EDGE as i32;
        let width = r.int("width", 1, edge)?;
        let height = r.int("height", 1, edge)?;
        let ts = r.int("tile_size", 8, 512)?;
        check(ts & (ts - 1) == 0, "タイル寸法が2の累乗ではありません")?;
        if version >= 7 {
            r.block("normal", |r| {
                r.int("algorithm", 1, 1)?;
                r.boolean("derive_from_height")?;
                r.float("strength", -256., 256.)?;
                r.int("edges", 0, 1)?;
                r.int("file_direction", 0, 1)?;
                Ok(())
            })?;
        }
        let user = if version >= USER_CHANNELS_VERSION {
            user_channels(&mut r, version)?
        } else {
            UserChannels::new()
        };
        if version >= SEAMS_VERSION {
            r.boolean("filter_seams")?;
        }
        if version >= BAKE_PRIORITY_VERSION {
            r.block("bake_priority", bake_priority)?;
        }
        let count = r.int("layer_count", 0, crate::MAX_DOCUMENT_LAYERS as i32)?;
        let layer_start = r.fields.len();
        Ok(Self {
            r,
            version,
            width,
            height,
            tile_size: ts,
            id,
            user,
            count,
            next: 0,
            layers: Vec::new(),
            ids: HashSet::new(),
            anchor_ids: HashMap::new(),
            ruler_ids: HashSet::new(),
            layer_start,
        })
    }
    /// 頭の項目（レイヤーより前）。
    pub fn head_fields(&self) -> &[NativeField] {
        &self.r.fields[..self.layer_start.min(self.r.fields.len())]
    }
    /// 次のレイヤーを読む（無ければ None）。読んだレイヤーの項目は `layer_fields` で見られる。
    pub fn next_layer(&mut self) -> Result<Option<usize>> {
        if self.next >= self.count {
            return Ok(None);
        }
        let i = self.next;
        self.layer_start = self.r.fields.len();
        let (v, w, h, ts) = (self.version, self.width, self.height, self.tile_size);
        let user = &self.user;
        let l = self
            .r
            .block(&format!("layers[{i}]"), |r| layer(r, v, w, h, ts, user))?;
        check(self.ids.insert(l.id), "レイヤーIDが重複しています")?;
        for id in &l.anchors {
            check(
                self.anchor_ids.insert(*id, i).is_none(),
                "Anchor IDが重複しています",
            )?;
        }
        for id in &l.rulers {
            check(self.ruler_ids.insert(*id), "定規のIDが重複しています")?;
        }
        self.layers.push(l);
        self.next += 1;
        Ok(Some(i as usize))
    }
    /// 今読んだレイヤーの項目。
    pub fn layer_fields(&self) -> &[NativeField] {
        &self.r.fields[self.layer_start..]
    }
    /// 今読んだレイヤーの `Bytes` の値（画素）を手放す（骨組みだけ残す）。
    pub fn drop_layer_values(&mut self) {
        let start = self.layer_start;
        let mut kept = 0;
        for k in start..self.r.fields.len() {
            if !matches!(self.r.fields[k].value, NativeValue::Bytes(_)) {
                self.r.fields.swap(start + kept, k);
                kept += 1;
            }
        }
        self.r.fields.truncate(start + kept);
    }
    /// レイヤーの後（手動の ID の色）と終わり、レイヤーをまたぐ決まり（親のグループ・Anchor・フィルターの ID）を確かめて、文書にする。
    pub fn finish(mut self) -> Result<NativeDocument> {
        check(
            self.next == self.count,
            "正本のレイヤーを読み終えていません",
        )?;
        let r = &mut self.r;
        if self.version >= MANUAL_ID_COLORS_VERSION && !r.at_end()? {
            r.block("manual_id_colors", |r| {
                r.blob_checked("tag", 4, |tag| {
                    check(tag == b"YLID", "末尾に未知のデータがあります")
                })?;
                let n = r.int("count", 1, 4096)?;
                check(is_hash(&r.string("binding")?), "手動ID色の指紋が不正です")?;
                let mut prev = -1;
                for i in 0..n {
                    r.block(&format!("colors[{i}]"), |r| {
                        let p = r.int("part", 0, 3999999)?;
                        check(p > prev, "手動ID色の部品番号の並びが不正です")?;
                        prev = p;
                        r.int("rgb", 0, 0xffffff)?;
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
        }
        check(
            r.at_end()?,
            "正本の末尾に未知のデータがあります。新しい読み手が必要です",
        )?;
        if let Some(parts) = r.parts.as_mut() {
            parts.finish()?;
        }
        let layers = &self.layers;
        let by_id: HashMap<_, _> = layers.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
        // 親子の確かめは上のレイヤーから下へ 1 回なめる（レイヤーの数 n に対して O(n)）。開いているグループの鎖を持ち、親でない所へ戻れば鎖を閉じる。
        // 親は子の上にある（位置が増える向きなので循環は起きない）・グループである・子が連続している（閉じたグループへ戻らない）・
        // 入れ子が上限以内。
        let mut open: Vec<usize> = Vec::new();
        for (i, l) in layers.iter().enumerate().rev() {
            if l.parent == [0; 16] {
                open.clear();
            } else {
                let p = *by_id
                    .get(&l.parent)
                    .ok_or_else(|| Error::InvalidData("親グループがありません".into()))?;
                check(
                    p > i && layers[p].kind == 3,
                    "親グループの位置・種類・循環が不正です",
                )?;
                while open.last() != Some(&p) {
                    check(open.pop().is_some(), "グループの子が連続していません")?;
                }
            }
            if l.kind == 3 {
                if open.len() >= yolu_core::MAX_GROUP_DEPTH {
                    return Err(Error::Budget(format!(
                        "グループの入れ子の上限は{}段です",
                        yolu_core::MAX_GROUP_DEPTH
                    )));
                }
                open.push(i);
            }
            for a in &l.references {
                check(
                    self.anchor_ids.get(a).copied() != Some(i as i32),
                    "自身のレイヤーのAnchorを参照しています",
                )?;
            }
        }
        validate_fields(&self.r.fields)?;
        let skeleton = self.r.keep == Keep::Skeleton;
        let fields = if skeleton {
            self.r
                .fields
                .into_iter()
                .filter(|f| !matches!(f.value, NativeValue::Bytes(_)))
                .collect()
        } else {
            self.r.fields
        };
        Ok(NativeDocument {
            version: self.version,
            id: self.id,
            width: self.width,
            height: self.height,
            tile_size: self.tile_size,
            layers: self.count as usize,
            fields,
            skeleton,
        })
    }
}
struct Reader<'a> {
    src: &'a mut dyn ByteSource,
    /// 版 26: `Bytes` の値はここから取る。
    parts: Option<&'a mut PartStream>,
    prefix: String,
    fields: Vec<NativeField>,
    keep: Keep,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let at = self.src.position();
        let prefix = &self.prefix;
        self.src.take(n).map_err(|e| match e {
            Error::InvalidData(why) if why == "正本が途中で切れています" => {
                Error::InvalidData(format!("正本が途中で切れています: {prefix} (位置{at})"))
            }
            other => other,
        })
    }
    fn at_end(&mut self) -> Result<bool> {
        self.src.at_end()
    }
    /// `Bytes` の値の中身が読めるか（版 26 の部分を長さだけで読むときは false）。
    fn content(&self) -> bool {
        self.parts.as_ref().is_none_or(|p| p.has_content())
    }
    fn add(&mut self, n: &str, value: NativeValue) {
        self.fields.push(NativeField {
            path: if self.prefix.is_empty() {
                n.into()
            } else {
                format!("{}.{n}", self.prefix)
            },
            value,
        });
    }
    fn block<T>(&mut self, n: &str, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let old = self.prefix.clone();
        self.prefix = if old.is_empty() {
            n.into()
        } else {
            format!("{old}.{n}")
        };
        let result = f(self);
        self.prefix = old;
        result
    }
    fn int(&mut self, n: &str, min: i32, max: i32) -> Result<i32> {
        let v = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
        check(
            (min..=max).contains(&v),
            format!(
                "{}.{} の値 {v} は未対応または範囲外です ({min}..{max})",
                self.prefix, n
            ),
        )?;
        self.add(n, NativeValue::Int(v));
        Ok(v)
    }
    fn byte(&mut self, n: &str) -> Result<u8> {
        let v = self.take(1)?[0];
        self.add(n, NativeValue::Byte(v));
        Ok(v)
    }
    fn boolean(&mut self, n: &str) -> Result<bool> {
        let v = self.take(1)?[0];
        check(v <= 1, format!("{}.{} の真偽値が不正です", self.prefix, n))?;
        self.add(n, NativeValue::Bool(v != 0));
        Ok(v != 0)
    }
    fn float(&mut self, n: &str, min: f64, max: f64) -> Result<f64> {
        let v = f64::from_le_bytes(self.take(8)?.try_into().unwrap());
        check(
            v.is_finite() && v >= min && v <= max,
            format!("{}.{} の数値が有限でないか範囲外です", self.prefix, n),
        )?;
        self.add(n, NativeValue::Float(v));
        Ok(v)
    }
    fn unit(&mut self, n: &str) -> Result<f64> {
        self.float(n, 0., 1.)
    }
    fn id(&mut self, n: &str, empty: bool) -> Result<[u8; 16]> {
        let v: [u8; 16] = self.take(16)?.try_into().unwrap();
        check(empty || v != [0; 16], "IDが空です")?;
        self.add(n, NativeValue::Guid(v));
        Ok(v)
    }
    /// `Bytes` の値（色・画素）。版 26 では部分から取る。骨組みの読みでは残さない（返すのは全部を残す読みと、確かめに要るときだけ）。
    fn blob(&mut self, n: &str, len: usize) -> Result<Option<Arc<[u8]>>> {
        self.blob_checked(n, len, |_| Ok(()))
    }
    /// `blob` に、中身の確かめを添える（中身の無い読みでは確かめない）。
    fn blob_checked(
        &mut self,
        n: &str,
        len: usize,
        verify: impl FnOnce(&[u8]) -> Result<()>,
    ) -> Result<Option<Arc<[u8]>>> {
        let content = self.content();
        let keep = self.keep == Keep::All;
        let at = self.src.position();
        let prefix = self.prefix.clone();
        let source: &mut dyn ByteSource = match self.parts.as_mut() {
            Some(p) => &mut **p,
            None => &mut *self.src,
        };
        let bytes = source.take(len).map_err(|e| match e {
            Error::InvalidData(why) if why == "正本が途中で切れています" => {
                Error::InvalidData(format!("正本が途中で切れています: {prefix} (位置{at})"))
            }
            other => other,
        })?;
        if content {
            verify(bytes)?;
        }
        if keep {
            let v: Arc<[u8]> = Arc::from(bytes);
            self.add(n, NativeValue::Bytes(v.clone()));
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }
    fn string(&mut self, n: &str) -> Result<String> {
        let len = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
        check((0..=4096).contains(&len), "文字列の長さが不正です")?;
        let s = std::str::from_utf8(self.take(len as usize)?)
            .map_err(|_| Error::InvalidData("文字列がUTF-8ではありません".into()))?
            .to_string();
        self.add(n, NativeValue::Text(s.clone()));
        Ok(s)
    }
}
struct Layer {
    id: [u8; 16],
    parent: [u8; 16],
    kind: i32,
    anchors: Vec<[u8; 16]>,
    references: Vec<[u8; 16]>,
    /// レイヤーの定規の ID。
    rulers: Vec<[u8; 16]>,
}
/// 版 22 のユーザーチャンネルの一覧: 数（版 22 は 1〜58で 0 の一覧は書かない。版 23 は 0〜58）、番号の昇順に番号（6〜63）・名前
/// （1〜128 文字、制御文字なし、標準の名前とも重ならない）・種類・色空間・既定の RGBA。
/// 版 33 の頭の `bake_priority`: 決め方・0〜1 の外のアイランドを焼かない・モデルの指紋・「焼かない」と「優先する」のアイランド（三角形の番号、狭義の昇順、
/// 両方に同じ番号を置かない）。一覧が両方とも空なら指紋も空、どちらかにあれば小文字の SHA-256 の 64 桁。
fn bake_priority(r: &mut Reader<'_>) -> Result<()> {
    r.int("rule", 0, 3)?;
    r.boolean("skip_outside")?;
    let binding = r.string("binding")?;
    let mut lists: [Vec<i32>; 2] = [Vec::new(), Vec::new()];
    for (name, list) in ["skip", "prefer"].into_iter().zip(lists.iter_mut()) {
        let count = r.int(
            &format!("{name}_count"),
            0,
            yolu_core::mesh_maps::MAX_OVERLAP_ISLANDS as i32,
        )?;
        for i in 0..count {
            let t = r.int(&format!("{name}[{i}]"), 0, 3_999_999)?;
            check(
                list.last().is_none_or(|p| *p < t),
                "ベイクの優先のアイランドの番号の並びが不正です",
            )?;
            list.push(t);
        }
    }
    check(
        lists[0].iter().all(|t| lists[1].binary_search(t).is_err()),
        "ベイクの優先の同じアイランドが「焼かない」と「優先する」の両方にあります",
    )?;
    check(
        if lists.iter().all(Vec::is_empty) {
            binding.is_empty()
        } else {
            is_hash(&binding)
        },
        "ベイクの優先のモデルの指紋が不正です",
    )
}
fn user_channels(r: &mut Reader<'_>, version: i32) -> Result<UserChannels> {
    let n = r.int(
        "user_channel_count",
        i32::from(version < PROCEDURAL_VERSION),
        64 - STANDARD_CHANNELS,
    )?;
    let mut names: HashSet<String> = [
        "Color",
        "Roughness",
        "Metallic",
        "Height",
        "Normal",
        "Emission",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut user = UserChannels::new();
    let mut previous = STANDARD_CHANNELS - 1;
    for i in 0..n {
        r.block(&format!("user_channels[{i}]"), |r| {
            let c = r.int("channel", STANDARD_CHANNELS, 63)?;
            check(c > previous, "ユーザーチャンネルの番号の並びが不正です")?;
            previous = c;
            let name = r.string("name")?;
            let chars = name.chars().count();
            check(
                (1..=128).contains(&chars) && !name.chars().any(char::is_control),
                "ユーザーチャンネルの名前が不正です",
            )?;
            check(names.insert(name), "チャンネルの名前が重複しています")?;
            let kind = r.int("kind", 0, 2)?;
            r.int("color_space", 0, 1)?;
            r.blob("default", 4)?;
            user.insert(c, kind);
            Ok(())
        })?;
    }
    Ok(user)
}
fn layer(
    r: &mut Reader<'_>,
    v: i32,
    w: i32,
    h: i32,
    ts: i32,
    user: &UserChannels,
) -> Result<Layer> {
    // 版 22 でユーザーチャンネルを置けるのは、チャンネルごとの合成・塗りつぶしの値・調整の対象・ラスターのチャンネル
    let channel_total = STANDARD_CHANNELS + user.len() as i32;
    let id = r.id("id", false)?;
    r.string("name")?;
    r.boolean("visible")?;
    r.unit("opacity")?;
    let blend = r.int("blend", 0, 26)?;
    let mut flags = 0;
    let mut ext = 0;
    let mut blends = Vec::new();
    if v >= 12 {
        flags = r.byte("attributes")?;
        let known = 3
            | if v >= 14 { 4 } else { 0 }
            | if v >= 16 { 8 } else { 0 }
            | if v >= 20 { 16 } else { 0 }
            | if v >= 21 { 32 } else { 0 }
            | if v >= PATHS_VERSION { 64 } else { 0 }
            | if v >= POINT_GRADIENT_VERSION { 128 } else { 0 };
        check(flags & !known == 0, "未知のレイヤー属性ビットです")?;
        if flags & 2 != 0 {
            r.int("locks", 1, 15)?;
        }
        // 続きの属性の印（ビット 7 のとき。ロックの直後）: ビット 0 塗りつぶしの点のグラデーション（版 29）、ビット 1 文字の値（版 30）
        if flags & 128 != 0 {
            ext = r.int("attributes_ext", 1, i32::MAX)?;
            let known_ext =
                1 | if v >= TEXT_VERSION { 2 } else { 0 } | if v >= RULERS_VERSION { 4 } else { 0 };
            check(ext & !known_ext == 0, "未知の続きのレイヤー属性ビットです")?;
        }
        if flags & 4 != 0 {
            let n = r.byte("channel_blend_count")?;
            check(
                (1..=channel_total).contains(&(n as i32)),
                "チャンネル合成数が不正です",
            )?;
            let mut seen = HashSet::new();
            for i in 0..n {
                r.block(&format!("channel_blends[{i}]"), |r| {
                    layer_channel(r, &mut seen, user)?;
                    let p = r.byte("parts")?;
                    check((1..=3).contains(&p), "未知の合成属性です")?;
                    if p & 1 != 0 {
                        blends.push(r.int("mode", 0, 26)?);
                    }
                    if p & 2 != 0 {
                        r.unit("opacity")?;
                    }
                    Ok(())
                })?;
            }
        }
    } else if v >= 5 {
        r.boolean("clipping")?;
    }
    let kind = if v >= 3 {
        r.int(
            "kind",
            0,
            if v >= 6 {
                3
            } else if v >= 4 {
                2
            } else {
                1
            },
        )?
    } else {
        0
    };
    check(
        kind == 3 || blend != 26 && !blends.contains(&26),
        "通過合成はグループだけに使えます",
    )?;
    let parent = if v >= 6 {
        r.id("parent", true)?
    } else {
        [0; 16]
    };
    let mut fills = HashSet::new();
    let mut images = HashSet::new();
    let mut gradients = HashSet::new();
    let mut references = Vec::new();
    if v >= 3 {
        let n = r.int("fill_count", 0, if kind == 1 { channel_total } else { 0 })?;
        for i in 0..n {
            r.block(&format!("fills[{i}]"), |r| {
                layer_channel(r, &mut fills, user)?;
                r.boolean("enabled")?;
                r.blob("rgba", 4)?;
                Ok(())
            })?;
        }
    }
    if flags & 8 != 0 {
        check(kind == 1, "塗りつぶし以外に画像投影があります")?;
        let n = r.int("image_count", 0, 6)?;
        for i in 0..n {
            r.block(&format!("images[{i}]"), |r| {
                let c = unique_channel(r, &mut images)?;
                check(fills.contains(&c), "画像に対応する塗りつぶし値がありません")?;
                r.id("resource_id", false)?;
                if v >= POINT_GRADIENT_VERSION {
                    r.boolean("anisotropic")?;
                }
                Ok(())
            })?;
        }
        let (_, default) = r.block("projection", |r| projection(r, v))?;
        check(n > 0 || !default, "空の画像投影ブロックです")?;
    }
    if flags & 32 != 0 {
        check(kind == 1, "塗りつぶし以外にグラデーションがあります")?;
        let n = r.int("gradient_count", 1, 6)?;
        let mut seen = HashSet::new();
        for i in 0..n {
            r.block(&format!("gradients[{i}]"), |r| {
                let c = unique_channel(r, &mut seen)?;
                check(
                    c != 4 && fills.contains(&c) && !images.contains(&c),
                    "グラデーションのチャンネル・塗りつぶし元が不正です",
                )?;
                check(
                    generator(r, v, &mut references)? == 5,
                    "塗りつぶしグラデーションは形状ジェネレーターが必要です",
                )?;
                Ok(())
            })?;
        }
        gradients.extend(seen);
    }
    if ext & 1 != 0 {
        check(kind == 1, "塗りつぶし以外に点のグラデーションがあります")?;
        let n = r.int("point_gradient_count", 1, 6)?;
        let mut seen = HashSet::new();
        for i in 0..n {
            r.block(&format!("point_gradients[{i}]"), |r| {
                let c = unique_channel(r, &mut seen)?;
                check(
                    c != 4 && fills.contains(&c) && !images.contains(&c) && !gradients.contains(&c),
                    "点のグラデーションのチャンネル・塗りつぶし元が不正です",
                )?;
                r.int("algorithm", 1, 1)?;
                let space = r.int("space", 0, 1)?;
                r.unit("spread")?;
                let count = r.int("point_count", 1, 64)?;
                for k in 0..count {
                    r.block(&format!("points[{k}]"), |r| {
                        r.float("x", -1e6, 1e6)?;
                        r.float("y", -1e6, 1e6)?;
                        let z = r.float("z", -1e6, 1e6)?;
                        check(space == 0 || z == 0., "UV の空間の点の z が 0 でありません")?;
                        r.blob("rgba", 4)?;
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
        }
    }
    if kind == 2 {
        r.block("adjustment", |r| {
            let t = r.int(
                "type",
                0,
                if v >= ADJUST_VERSION {
                    ADJUST_KIND_MAX
                } else {
                    2
                },
            )?;
            // 3〜63 は Unity 版の将来のために空けてある（Rust 版は使わない）
            check(!(3..ADJUST_KIND_MIN).contains(&t), "未知の調整の種類です")?;
            r.int("algorithm", 1, 1)?;
            let mut p = Vec::new();
            for k in [
                "input_black",
                "input_white",
                "gamma",
                "output_black",
                "output_white",
                "hue",
                "saturation",
                "lightness",
            ] {
                p.push(r.float(k, -f64::MAX, f64::MAX)?);
            }
            if t == 1 {
                levels(&p[..5])?;
            }
            if t == 2 {
                check(
                    (-180. ..=180.).contains(&p[5])
                        && (-1. ..=1.).contains(&p[6])
                        && (-1. ..=1.).contains(&p[7]),
                    "色相・彩度・明度が範囲外です",
                )?;
            }
            if t >= ADJUST_KIND_MIN {
                // 64 からの種類は 8 つの値を使わない（既定のまま）。値は種類ごとの欄に続く
                check(
                    p == [0., 1., 1., 0., 1., 0., 0., 0.],
                    "未使用の調整の値が変更されています",
                )?;
                r.block("detail", |r| color_adjust(r, v, t))?;
            }
            let n = r.int("channel_count", 0, channel_total)?;
            let mut seen = HashSet::new();
            for i in 0..n {
                r.block(&format!("channels[{i}]"), |r| {
                    let c = layer_channel(r, &mut seen, user)?;
                    // 色だけの種類（色相/彩度・グラデーションマップ・カラーバランス）は色のチャンネルだけ。
                    // トーンカーブ・明るさ/コントラスト・2 値化・ポスタリゼーションは法線に当てない
                    let colour_only = matches!(t, 2 | 64 | 66);
                    let not_normal = matches!(t, 65 | 67..=69);
                    check(
                        (!colour_only || c == 0 || c == 5 || user.get(&c) == Some(&0))
                            && (!not_normal || (c != 4 && user.get(&c) != Some(&2))),
                        "調整対象のチャンネルが不正です",
                    )
                })?;
            }
            Ok(())
        })?;
    }
    // 画素はラスターレイヤーと、パスの一覧を持つ塗りつぶしレイヤー（パスの画素。版 27）
    let n = r.int(
        "channel_count",
        0,
        if kind == 0 || (kind == 1 && flags & 64 != 0) {
            channel_total
        } else {
            0
        },
    )?;
    let mut channels = HashSet::new();
    let mut enabled = HashSet::new();
    for i in 0..n {
        r.block(&format!("channels[{i}]"), |r| {
            let c = layer_channel(r, &mut channels, user)?;
            if r.boolean("enabled")? {
                enabled.insert(c);
            }
            tiles(r, w, h, ts, false)
        })?;
    }
    let mask = v >= 2 && r.boolean("has_mask")?;
    if mask {
        r.block("mask", |r| {
            r.boolean("enabled")?;
            r.boolean("inverted")?;
            r.unit("density")?;
            tiles(r, w, h, ts, true)
        })?;
    }
    let surface = v >= 8 && r.boolean("has_surface_path")?;
    if surface {
        check(kind == 0, "パスはラスターレイヤーに限ります")?;
        r.block("surface_path", |r| path(r, v, true, &channels, &enabled))?;
    }
    if v >= 9 && r.boolean("has_filters")? {
        r.block("filters", |r| filters(r, v, true, &mut references))?;
        if mask {
            r.block("mask.filters", |r| filters(r, v, false, &mut references))?;
        }
    }
    let canvas = v >= 10 && r.boolean("has_canvas_path")?;
    if canvas {
        check(!surface && kind == 0, "パスの種類またはレイヤーが不正です")?;
        r.block("canvas_path", |r| path(r, v, false, &channels, &enabled))?;
    }
    let mut anchors = Vec::new();
    if flags & 16 != 0 {
        let f = r.byte("anchor_flags")?;
        check(
            (1..=3).contains(&f) && (f & 2 == 0 || mask),
            "Anchorの属性またはマスクが不正です",
        )?;
        for (bit, name) in [(1, "anchor"), (2, "mask.anchor")] {
            if f & bit != 0 {
                r.block(name, |r| {
                    anchors.push(r.id("id", false)?);
                    let s = r.string("name")?;
                    check(
                        !s.trim().is_empty() && s.encode_utf16().count() <= 128,
                        "Anchorの名前が不正です",
                    )
                })?;
            }
        }
    }
    if flags & 64 != 0 {
        // 一覧はラスターと塗りつぶしレイヤー（塗りつぶしレイヤーのパスは一覧の形だけ）
        check(
            !surface && !canvas && (kind == 0 || kind == 1),
            "パスの一覧と 1 本のパスは両方を持てません（パスはラスターか塗りつぶしレイヤーに限ります）",
        )?;
        r.block("paths", |r| path_list(r, v, &channels, &enabled))?;
    }
    if ext & 2 != 0 {
        check(
            kind == 0 && !surface && !canvas && flags & 64 == 0,
            "テキストの値はパスの無いラスターのレイヤーだけが持てます",
        )?;
        check(
            channels.contains(&0) && enabled.contains(&0),
            "テキストレイヤーに有効な Color の画素がありません",
        )?;
        r.block("text", text)?;
    }
    let mut rulers = Vec::new();
    if ext & 4 != 0 {
        let n = r.int("ruler_count", 1, yolu_core::MAX_RULERS_PER_LAYER as i32)?;
        for i in 0..n {
            rulers.push(r.block(&format!("rulers[{i}]"), ruler)?);
        }
    }
    Ok(Layer {
        id,
        parent,
        kind,
        anchors,
        references,
        rulers,
    })
}
/// 定規 1 つ（版 35）。値の確かめは core の `Ruler::validate` と同じ（座標の範囲は欄ごとに読み、残りは core の型で確かめる）。
/// 返すのは ID。
fn ruler(r: &mut Reader<'_>) -> Result<[u8; 16]> {
    use yolu_core::glam::{DVec2, DVec3};
    use yolu_core::{Ruler, RulerId, RulerKind, RulerPlace, RulerScope};
    let id = r.id("id", false)?;
    let kind = RulerKind::from_index(r.byte("kind")?)
        .ok_or_else(|| Error::InvalidData("定規の種類が不正です".into()))?;
    let space = r.byte("space")?;
    check(space <= 1, "定規の空間が不正です")?;
    let flags = r.byte("flags")?;
    check(flags & !31 == 0, "未知の定規の印のビットです")?;
    let scope = RulerScope::from_index(r.byte("scope")?)
        .ok_or_else(|| Error::InvalidData("定規の表示の範囲が不正です".into()))?;
    let lines = r.byte("lines")?;
    let max = if space == 0 {
        yolu_core::rulers::MAX_CANVAS_COORD
    } else {
        yolu_core::rulers::MAX_MODEL_COORD
    };
    let point = |r: &mut Reader<'_>, names: [&str; 3]| -> Result<DVec3> {
        let x = r.float(names[0], -max, max)?;
        let y = r.float(names[1], -max, max)?;
        let z = if space == 1 {
            r.float(names[2], -max, max)?
        } else {
            0.0
        };
        Ok(DVec3::new(x, y, z))
    };
    let a = point(r, ["a_x", "a_y", "a_z"])?;
    let b = point(r, ["b_x", "b_y", "b_z"])?;
    let place = if space == 0 {
        RulerPlace::Canvas {
            a: DVec2::new(a.x, a.y),
            b: DVec2::new(b.x, b.y),
        }
    } else {
        let up = DVec3::new(
            r.float("up_x", -max, max)?,
            r.float("up_y", -max, max)?,
            r.float("up_z", -max, max)?,
        );
        RulerPlace::Model { a, b, up }
    };
    let ruler = Ruler {
        id: RulerId(1),
        kind,
        place,
        two_points: flags & 4 != 0,
        lines,
        line_symmetry: flags & 8 != 0,
        see_through: flags & 16 != 0,
        // 表示の印（ビット 0）は確かめに関係しない（読み込みの値は core の読み手が持つ）
        visible: true,
        scope,
        snap: flags & 2 != 0,
    };
    ruler
        .validate()
        .map_err(|e| Error::InvalidData(format!("{}の定規が不正です: {e}", r.prefix)))?;
    Ok(id)
}
/// テキストレイヤーの値（版 30。範囲は `yolu_core::text` の値の検査と同じ）。
fn text(r: &mut Reader<'_>) -> Result<()> {
    use yolu_core::text as t;
    r.int("algorithm", 1, 1)?;
    let content = r.string("content")?;
    check(
        content.len() <= t::MAX_TEXT_BYTES
            && !content
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "テキストの文が長すぎるか、改行とタブのほかの制御文字があります",
    )?;
    if r.int("font_kind", 0, 1)? == 0 {
        let name = r.string("font_name")?;
        check(
            !name.is_empty()
                && name.len() <= t::MAX_FONT_NAME
                && name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
            "同梱のフォントの名前が不正です",
        )?;
    } else {
        let path = r.string("font_path")?;
        check(
            !path.is_empty()
                && path.len() <= t::MAX_FONT_PATH_BYTES
                && !path.chars().any(char::is_control),
            "フォントのファイルの道が不正です",
        )?;
        r.int("font_index", 0, i32::MAX)?;
        let sha = r.string("font_sha256")?;
        check(is_hash(&sha), "フォントのファイルの SHA-256 が不正です")?;
        for name in ["font_family", "font_postscript"] {
            let v = r.string(name)?;
            check(
                v.len() <= t::MAX_FONT_FAMILY_BYTES && !v.chars().any(char::is_control),
                "フォントの名前が不正です",
            )?;
        }
        r.int("font_weight", 1, 1000)?;
        r.boolean("font_italic")?;
    }
    r.float("size", t::MIN_SIZE, t::MAX_SIZE)?;
    r.blob("rgba", 4)?;
    r.float("line_height", t::MIN_LINE_HEIGHT, t::MAX_LINE_HEIGHT)?;
    r.float(
        "letter_spacing",
        t::MIN_LETTER_SPACING,
        t::MAX_LETTER_SPACING,
    )?;
    r.int("align", 0, 2)?;
    r.float("x", -t::MAX_COORDINATE, t::MAX_COORDINATE)?;
    r.float("y", -t::MAX_COORDINATE, t::MAX_COORDINATE)?;
    r.float("rotation", -360., 360.)?;
    r.float("wrap_width", 0., t::MAX_COORDINATE)?;
    Ok(())
}
/// 標準のチャンネルだけの項目（塗りつぶしの画像・グラデーション、フィルター、パスのマテリアル）。
fn unique_channel(r: &mut Reader<'_>, seen: &mut HashSet<i32>) -> Result<i32> {
    let c = r.int("channel", 0, STANDARD_CHANNELS - 1)?;
    check(seen.insert(c), "チャンネルが重複しています")?;
    Ok(c)
}
/// ユーザーチャンネルも置ける項目。版 21 までは標準だけ、版 22 は一覧にある番号だけ。
fn layer_channel(r: &mut Reader<'_>, seen: &mut HashSet<i32>, user: &UserChannels) -> Result<i32> {
    let max = if user.is_empty() {
        STANDARD_CHANNELS - 1
    } else {
        63
    };
    let c = r.int("channel", 0, max)?;
    check(
        c < STANDARD_CHANNELS || user.contains_key(&c),
        format!(
            "{}.channel {c} はプロジェクトのチャンネルの一覧にありません",
            r.prefix
        ),
    )?;
    check(seen.insert(c), "チャンネルが重複しています")?;
    Ok(c)
}
fn tiles(r: &mut Reader<'_>, w: i32, h: i32, ts: i32, mask: bool) -> Result<()> {
    let cols = (w + ts - 1) / ts;
    let rows = (h + ts - 1) / ts;
    let n = r.int("tile_count", 0, cols * rows)?;
    let mut seen = HashSet::new();
    for i in 0..n {
        r.block(&format!("tiles[{i}]"), |r| {
            let x = r.int("x", 0, cols - 1)?;
            let y = r.int("y", 0, rows - 1)?;
            check(seen.insert((x, y)), "タイルが重複しています")?;
            let len = r.int("length", ts * ts * 4, ts * ts * 4)?;
            r.blob_checked("rgba", len as usize, |b| {
                check(
                    !mask || b.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]),
                    "マスクのRGBは0でなければなりません",
                )
            })?;
            Ok(())
        })?;
    }
    Ok(())
}
fn levels(p: &[f64]) -> Result<()> {
    check(
        p[0] >= 0.
            && p[1] <= 1.
            && p[1] - p[0] >= 1. / 255.
            && (0.1..=9.99).contains(&p[2])
            && (0. ..=1.).contains(&p[3])
            && (0. ..=1.).contains(&p[4]),
        "レベル補正が範囲外です",
    )
}
fn volume(r: &mut Reader<'_>, falloff: bool) -> Result<Vec<f64>> {
    let mut p = Vec::new();
    for (names, min, max) in [
        (["center_x", "center_y", "center_z"], -1e6, 1e6),
        (["rotation_x", "rotation_y", "rotation_z"], -360., 360.),
        (["size_x", "size_y", "size_z"], 1e-6, 1e6),
    ] {
        for n in names {
            p.push(r.float(n, min, max)?);
        }
    }
    if falloff {
        p.push(r.unit("falloff")?);
    }
    Ok(p)
}
/// 投影の欄。返すのは (種類, 既定のままか)。
fn projection(r: &mut Reader<'_>, v: i32) -> Result<(i32, bool)> {
    r.int("algorithm", 1, 1)?;
    let mode = r.int("mode", 0, if v >= 17 { 5 } else { 4 })?;
    let wrap = r.int("wrap", 0, if v >= 17 { 2 } else { 1 })?;
    let u = r.float("tile_u", 1e-3, 1e4)?;
    let vv = r.float("tile_v", 1e-3, 1e4)?;
    let ou = r.float("offset_u", -1e4, 1e4)?;
    let ov = r.float("offset_v", -1e4, 1e4)?;
    let rot = r.float("rotation", -360., 360.)?;
    let blend = r.unit("blend_width")?;
    let p = r.block("placement", |r| volume(r, false))?;
    if mode == 5 {
        r.unit("depth_hardness")?;
        r.float("backface_angle", 0., 180.)?;
        r.unit("backface_hardness")?;
    }
    Ok((
        mode,
        mode == 0
            && wrap == 0
            && u == 1.
            && vv == 1.
            && ou == 0.
            && ov == 0.
            && rot == 0.
            && blend == 0.3
            && p == [0., 0., 0., 0., 0., 0., 1., 1., 1.],
    ))
}
fn generator(r: &mut Reader<'_>, v: i32, refs: &mut Vec<[u8; 16]>) -> Result<i32> {
    let t = r.int(
        "type",
        0,
        if v >= EFFECTS_VERSION {
            IMAGE_KIND
        } else if v >= PROCEDURAL_VERSION {
            PROCEDURAL_KIND_MAX
        } else if v >= 20 {
            7
        } else if v >= 15 {
            6
        } else if v >= 13 {
            5
        } else {
            4
        },
    )?;
    // 8〜63 は Unity 版の将来のために空けてある（Rust 版は使わない）
    check(
        !(8..PROCEDURAL_KIND_MIN).contains(&t),
        "未知のジェネレーターの種類です",
    )?;
    let algorithm = r.int("algorithm", 1, if t == 5 && v >= 21 { 2 } else { 1 })?;
    let low = r.unit("low")?;
    let high = r.unit("high")?;
    check(
        high - low >= 0.001,
        "ジェネレーターのレベル幅が不足しています",
    )?;
    r.unit("softness")?;
    r.boolean("invert")?;
    let noise_amount = r.unit("noise_amount")?;
    let noise_scale = r.float("noise_scale", 0.001, 1.)?;
    let noise_seed = r.int("noise_seed", i32::MIN, i32::MAX)?;
    let noise_space = r.int("noise_space", 0, 1)?;
    r.int("blend", 0, 6)?;
    let balance = r.unit("balance")?;
    let axis = r.int("axis", 0, 2)?;
    let x = r.float("direction_x", -1e6, 1e6)?;
    let y = r.float("direction_y", -1e6, 1e6)?;
    let z = r.float("direction_z", -1e6, 1e6)?;
    let bent = r.boolean("bent_normal")?;
    check(
        (t == 1 || balance == 0.5) && (t == 2 || axis == 1),
        "ジェネレーター固有でない属性が変更されています",
    )?;
    check(
        if t == 4 {
            x * x + y * y + z * z >= 1e-12
        } else {
            x == 0. && y == 1. && z == 0. && !bent
        },
        "ジェネレーターの方向が不正です",
    )?;
    let n = r.int("pin_count", 0, 8)?;
    let mut seen = HashSet::new();
    let candidates: &[i32] = match t {
        0 => &[3, 1],
        1 => &[2, 3, 1],
        2 | 5 | 7 => &[1],
        3 => &[4, 1],
        6 => &[7, 1],
        IMAGE_KIND => &[],
        66 => &[],
        68 => &[0],
        69 => &[3, 2, 1, 4],
        ISLAND_KIND => &[],
        PROCEDURAL_KIND_MIN.. => &[1, 0],
        _ => &[0, 8, 1],
    };
    for i in 0..n {
        r.block(&format!("pins[{i}]"), |r| {
            let k = r.int("kind", 0, 9)?;
            check(
                seen.insert(k) && candidates.contains(&k),
                "ジェネレーターが使わないか重複したメッシュマップです",
            )?;
            check(
                is_hash(&r.string("key")?),
                "メッシュマップの条件キーが不正です",
            )
        })?;
    }
    if t == 5 {
        r.block("volume", |r| {
            r.int("shape", 0, 2)?;
            volume(r, true)?;
            Ok(())
        })?;
        if algorithm == 2 {
            r.block("ramp", ramp)?;
        }
    }
    if t == 6 {
        r.int("tolerance", 0, 255)?;
        let n = r.int("color_count", 0, 32)?;
        let mut seen = HashSet::new();
        for i in 0..n {
            check(
                seen.insert(r.int(&format!("colors[{i}]"), 0, 0xffffff)?),
                "ジェネレーターのID色が重複しています",
            )?;
        }
    }
    if t == 7 {
        refs.push(r.id("anchor_id", true)?);
        check(
            r.int("anchor_channel", 0, 5)? != 4,
            "AnchorはNormalを読めません",
        )?;
        r.int("anchor_read", 0, 1)?;
    }
    if (PROCEDURAL_KIND_MIN..=PROCEDURAL_KIND_MAX).contains(&t) {
        r.block("procedural", |r| procedural(r, t))?;
    }
    if t == IMAGE_KIND {
        // 重ねるノイズを持たない（ノイズ・グランジと同じ）
        check(
            noise_amount == 0. && noise_scale == 0.05 && noise_seed == 0 && noise_space == 0,
            "画像のジェネレーターは重ねるノイズを持ちません",
        )?;
        // 種類ごとの欄は、版 28 のほかの Generator（66・68・69）と同じ `effect` の塊に置く
        r.block("effect", |r| {
            r.id("resource_id", true)?;
            let (mode, _) = r.block("projection", |r| projection(r, v))?;
            check(mode != 5, "画像のジェネレーターはデカールに投影できません")?;
            r.int("component", 0, 4)?;
            Ok(())
        })?;
    } else if t == ISLAND_KIND {
        r.block("effect", island_effect)?;
    } else if t > PROCEDURAL_KIND_MAX {
        r.block("effect", |r| generator_effect(r, t))?;
    }
    Ok(t)
}
/// Generator の種類 70（画像。正本の版 28。66 模様・67 アイランドごとのばらつき・68 ライト・69 マスクの組み立ても同じ版）。
const IMAGE_KIND: i32 = 70;
/// Generator の種類 67（アイランドごとのばらつき。正本の版 28）。
const ISLAND_KIND: i32 = 67;
/// 版 28 の Generator の種類ごとの欄（`effect` の塊の中）。66: 形・繰り返し・角度・太さ・ぼかし・ずれ。68: 水平の角度・高さ・回り込み・底上げ。
/// 69: 曲率・AO・位置・厚みの塊（重み・位置・コントラスト・反転）と合わせ方（67 は `island_effect`）。
fn generator_effect(r: &mut Reader<'_>, t: i32) -> Result<()> {
    match t {
        66 => {
            r.int("shape", 0, 4)?;
            r.float("scale", 1., 512.)?;
            r.float("angle", 0., 360.)?;
            r.unit("width")?;
            r.unit("softness")?;
            r.unit("offset_u")?;
            r.unit("offset_v")?;
        }
        68 => {
            r.float("azimuth", 0., 360.)?;
            r.float("elevation", 0., 90.)?;
            r.unit("softness")?;
            r.unit("ambient")?;
        }
        _ => {
            for map in ["curvature", "ambient_occlusion", "position", "thickness"] {
                r.block(map, |r| {
                    r.unit("weight")?;
                    r.unit("level")?;
                    r.unit("contrast")?;
                    r.boolean("invert")?;
                    Ok(())
                })?;
            }
            r.int("combine", 0, 2)?;
        }
    }
    Ok(())
}
/// アイランドごとのばらつき（67）の欄（`effect` の塊の中）: シード・最小・最大（最小 ≤ 最大）。
fn island_effect(r: &mut Reader<'_>) -> Result<()> {
    r.int("seed", i32::MIN, i32::MAX)?;
    let min = r.unit("min")?;
    let max = r.unit("max")?;
    check(
        min <= max,
        "アイランドごとのばらつきの最小が最大を超えています",
    )
}
/// Rust 版だけの Generator の種類の番号（ノイズ 64・グランジ 65）。
const PROCEDURAL_KIND_MIN: i32 = 64;
const PROCEDURAL_KIND_MAX: i32 = 65;
/// ノイズ・グランジの欄: 空間・模様の大きさ・シード・回転・にじみ・トライプラナーの幅に、ノイズ（64）は基底・セルの出力・重ね方・
/// オクターブ・ラクナリティ・ゲイン、グランジ（65）はプリセット。
fn procedural(r: &mut Reader<'_>, t: i32) -> Result<()> {
    r.int("space", 0, 2)?;
    r.float("scale", 0.001, 1.)?;
    r.int("seed", i32::MIN, i32::MAX)?;
    for axis in ["rotation_x", "rotation_y", "rotation_z"] {
        r.float(axis, -360., 360.)?;
    }
    r.unit("bleed")?;
    r.unit("blend_width")?;
    if t == PROCEDURAL_KIND_MIN {
        let basis = r.int("basis", 0, 2)?;
        let cell = r.int("cell_output", 0, 2)?;
        check(basis == 2 || cell == 0, "セルの出力はWorley専用です")?;
        r.int("fractal", 0, 2)?;
        r.int("octaves", 1, 8)?;
        r.float("lacunarity", 1., 4.)?;
        r.unit("gain")?;
    } else {
        r.int("preset", 0, 10)?;
    }
    Ok(())
}
/// Rust 版だけの調整・フィルターの種類の番号（グラデーションマップ 64〜ポスタリゼーション 69）。
const ADJUST_KIND_MIN: i32 = 64;
const ADJUST_KIND_MAX: i32 = 69;
/// 版 28 のフィルターの段の種類の一番大きい番号（70 ヒストグラムスキャン〜79 グロー）。
const FILTER_KIND_MAX: i32 = 79;
/// 版 28 のフィルターの段の種類ごとの欄（`effect` の塊の中）。返すのは到達半径（画素。実数の長さは切り上げ）。
/// 70: 位置・コントラスト。71: 範囲・位置。72: 長さ・取る数・合わせ方・ノイズの大きさ・シード。73: 角度・長さ。74: 長さ・ノイズの大きさ・シード。
/// 75: 向き・半径。76: 幅・しきい値。77・78: 半径。79: しきい値・半径・強さ。
fn effect_filter(r: &mut Reader<'_>, t: i32) -> Result<i32> {
    let length = |v: f64| v.ceil() as i32;
    Ok(match t {
        70 => {
            r.unit("position")?;
            r.unit("contrast")?;
            0
        }
        71 => {
            r.unit("range")?;
            r.unit("position")?;
            0
        }
        72 => {
            let intensity = r.float("intensity", 0., 64.)?;
            r.int("samples", 1, 32)?;
            r.int("mode", 0, 2)?;
            r.float("scale", 1., 256.)?;
            r.int("seed", i32::MIN, i32::MAX)?;
            length(intensity)
        }
        73 => {
            r.float("angle", 0., 360.)?;
            length(r.float("distance", 0., 256.)?)
        }
        74 => {
            let intensity = r.float("intensity", 0., 128.)?;
            r.float("scale", 1., 256.)?;
            r.int("seed", i32::MIN, i32::MAX)?;
            length(intensity)
        }
        75 => {
            r.int("mode", 0, 1)?;
            r.int("radius", 1, 64)?
        }
        76 => {
            let width = r.int("width", 1, 16)?;
            r.unit("threshold")?;
            width + 1
        }
        77 => r.int("radius", 1, 256)?,
        78 => r.int("radius", 1, 16)?,
        _ => {
            r.unit("threshold")?;
            let radius = r.int("radius", 1, 256)?;
            r.float("intensity", 0., 4.)?;
            radius
        }
    })
}
/// 64 からの調整・フィルターの種類ごとの欄（調整レイヤーは `detail`、フィルターの段は `adjust` のブロックの中）。
/// 64: 逆向き・ランプ。65: 合成・R・G・B の 4 本のカーブ。66: 範囲ごとの 3 本のスライダーと輝度を保つ。
/// 67: 明るさ・コントラスト。68: しきい値。69: 階調。
fn color_adjust(r: &mut Reader<'_>, v: i32, t: i32) -> Result<()> {
    match t {
        64 => {
            r.boolean("reverse")?;
            let colors = r.block("ramp", |r| {
                let colors = ramp(r)?;
                Ok(colors)
            })?;
            if v >= MIXING_VERSION {
                gradient_mixing(r, colors)?;
            }
        }
        65 => {
            for name in ["composite", "red", "green", "blue"] {
                curve_points(r, name)?;
            }
        }
        66 => {
            for range in ["shadows", "midtones", "highlights"] {
                for axis in ["cyan_red", "magenta_green", "yellow_blue"] {
                    r.float(&format!("{range}_{axis}"), -100., 100.)?;
                }
            }
            r.boolean("preserve_luminosity")?;
        }
        67 => {
            r.float("brightness", -150., 150.)?;
            r.float("contrast", -50., 100.)?;
        }
        68 => {
            r.int("level", 1, 255)?;
        }
        _ => {
            r.int("levels", 2, 255)?;
        }
    }
    Ok(())
}
/// 値のカーブの点（数 2〜16、x は 0 から 1 まで昇順で間隔 0.02 − 1e-6 以上、y は 0〜1）。欄の名前は `{name}_count`・`{name}[i].x`・`{name}[i].y`。
fn curve_points(r: &mut Reader<'_>, name: &str) -> Result<()> {
    let n = r.int(&format!("{name}_count"), 2, 16)?;
    let mut previous = -1.;
    for i in 0..n {
        r.block(&format!("{name}[{i}]"), |r| {
            let p = r.unit("x")?;
            check(
                i == 0 || p - previous >= 0.02 - 1e-6,
                "ランプの点が昇順でないか近すぎます",
            )?;
            previous = p;
            check(
                (i != 0 || p == 0.) && (i != n - 1 || p == 1.),
                "カーブは0から1まで必要です",
            )?;
            r.unit("y")?;
            Ok(())
        })?;
    }
    Ok(())
}
/// グラデーションマップの混色の欄（正本の版 25。ランプのあと）: 混色モード（0 通常・1 知覚的・2 リニア）、輝度の補正（0〜4。知覚的でなければ
/// 既定の 3）、区間の数（色の分岐点の数 − 1）と、区間ごとの `enabled` と、あれば混合率曲線 `curve`。
fn gradient_mixing(r: &mut Reader<'_>, colors: i32) -> Result<()> {
    let mode = r.int("mix", 0, 2)?;
    let correction = r.int("luminance", 0, 4)?;
    check(
        mode == 1 || correction == 3,
        "輝度の補正は知覚的な混色のときだけです",
    )?;
    let n = r.int("segment_count", 1, 31)?;
    check(
        n == colors - 1,
        "混合率曲線の区間の数が色の分岐点と合いません",
    )?;
    for i in 0..n {
        r.block(&format!("segments[{i}]"), |r| {
            if r.boolean("enabled")? {
                curve_points(r, "curve")?;
            }
            Ok(())
        })?;
    }
    Ok(())
}
/// ランプ（色・不透明度の分岐点と値のカーブ）。色の分岐点の数を返す。
fn ramp(r: &mut Reader<'_>) -> Result<i32> {
    let mut colors = 0;
    for (kind, max) in [("colors", 32), ("opacities", 32)] {
        let n = r.int(&format!("{kind}_count"), 2, max)?;
        if kind == "colors" {
            colors = n;
        }
        let mut previous = -1.;
        for i in 0..n {
            r.block(&format!("{kind}[{i}]"), |r| {
                let p = r.unit("position")?;
                check(
                    i == 0 || p - previous >= 0.0001,
                    "ランプの点が昇順でないか近すぎます",
                )?;
                previous = p;
                if kind == "colors" {
                    r.blob("rgb", 3)?;
                } else {
                    r.unit("opacity")?;
                }
                r.float("midpoint", 0.01, 0.99)?;
                Ok(())
            })?;
        }
    }
    curve_points(r, "curve")?;
    Ok(colors)
}
fn filters(r: &mut Reader<'_>, v: i32, content: bool, refs: &mut Vec<[u8; 16]>) -> Result<()> {
    let n = r.int("count", 0, 32)?;
    let mut ids = HashSet::new();
    let mut halos = [0; 6];
    for i in 0..n {
        r.block(&format!("items[{i}]"), |r| {
            check(
                ids.insert(r.id("id", false)?),
                "フィルターIDが重複しています",
            )?;
            let t = r.int(
                "type",
                0,
                if v >= EFFECTS_VERSION {
                    FILTER_KIND_MAX
                } else if v >= ADJUST_VERSION {
                    ADJUST_KIND_MAX
                } else if v >= 11 {
                    6
                } else {
                    5
                },
            )?;
            // 7〜63 は Unity 版の将来のために空けてある（Rust 版は使わない）
            check(
                !(7..ADJUST_KIND_MIN).contains(&t),
                "未知のフィルターの種類です",
            )?;
            r.int("algorithm", 1, 1)?;
            let active = r.boolean("enabled")?;
            let strength = r.unit("strength")?;
            let mut channels = HashSet::new();
            if content {
                let n = r.int("channel_count", 1, 6)?;
                for i in 0..n {
                    r.block(&format!("channels[{i}]"), |r| {
                        unique_channel(r, &mut channels)
                    })?;
                }
            }
            let radius = r.int("radius", 0, 256)?;
            let amount = r.float("amount", 0., 5.)?;
            let threshold = r.int("threshold", 0, 255)?;
            let seed = r.int("seed", i32::MIN, i32::MAX)?;
            let mono = r.boolean("monochrome")?;
            let mut p = Vec::new();
            for name in [
                "input_black",
                "input_white",
                "gamma",
                "output_black",
                "output_white",
            ] {
                p.push(r.float(name, -f64::MAX, f64::MAX)?);
            }
            check(
                match t {
                    0 => (1..=256).contains(&radius),
                    1 => (1..=64).contains(&radius),
                    _ => radius == 0,
                },
                "フィルターの半径が不正です",
            )?;
            check(
                (t == 1 || t == 2 && amount <= 1. || amount == 0.)
                    && (t == 1 || threshold == 0)
                    && (t == 2 || seed == 0 && !mono),
                "フィルターのパラメーターが不正です",
            )?;
            if t == 3 {
                levels(&p)?;
            } else {
                check(
                    p == [0., 1., 1., 0., 1.],
                    "未使用のレベル補正値が変更されています",
                )?;
            }
            if channels.contains(&4) {
                check(t == 0, "Normalに適用できないフィルターです")?;
            }
            if t == 2 && !mono {
                check(
                    content && !channels.iter().any(|c| [1, 2, 3].contains(c)),
                    "スカラーチャンネルにカラーNoiseを適用できません",
                )?;
            }
            if t == 6 {
                r.block("generator", |r| generator(r, v, refs))?;
            }
            if (ADJUST_KIND_MIN..=ADJUST_KIND_MAX).contains(&t) {
                // グラデーションマップ・カラーバランスは色のチャンネルだけ（スカラーのチャンネルとマスクには置けない）
                if matches!(t, 64 | 66) {
                    check(
                        content && !channels.iter().any(|c| [1, 2, 3].contains(c)),
                        "スカラーチャンネルとマスクに色だけの調整を適用できません",
                    )?;
                }
                r.block("adjust", |r| color_adjust(r, v, t))?;
            }
            // 版 28 の種類（70〜79）の欄と、その到達半径
            let mut reach = 0;
            if t > ADJUST_KIND_MAX {
                // ヒストグラム・モルフォロジー・エッジ検出はスカラーのチャンネルとマスクだけ、グローは色のチャンネルだけ
                if matches!(t, 70 | 71 | 75 | 76) {
                    check(
                        !content || channels.iter().all(|c| [1, 2, 3].contains(c)),
                        "色のチャンネルにスカラーだけのフィルターを適用できません",
                    )?;
                }
                if t == 79 {
                    check(
                        content && !channels.iter().any(|c| [1, 2, 3].contains(c)),
                        "スカラーチャンネルとマスクに色だけのフィルターを適用できません",
                    )?;
                }
                reach = r.block("effect", |r| effect_filter(r, t))?;
            }
            if active && strength > 0. {
                for (channel, halo) in halos.iter_mut().enumerate() {
                    if !content || channels.contains(&(channel as i32)) {
                        *halo += radius + reach;
                        check_budget(
                            *halo <= 512,
                            "フィルタースタックの到達半径が512を超えています",
                        )?;
                    }
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}
/// 1 本のパス。点の数を返す（版 27 の一覧の拡張が点の番号を確かめるのに使う）。
fn path(
    r: &mut Reader<'_>,
    v: i32,
    surface: bool,
    channels: &HashSet<i32>,
    enabled: &HashSet<i32>,
) -> Result<i32> {
    r.int("algorithm", 1, 1)?;
    r.id("id", true)?;
    let channel = r.int("channel", 0, 5)?;
    if surface {
        let s = r.string("model_fingerprint")?;
        check(
            !s.is_empty() && s.encode_utf16().count() <= 128,
            "パスのモデル指紋が不正です",
        )?;
    }
    r.block("brush", |r| {
        let radius = r.float("radius", 0., if surface { 1e6 } else { 4096. })?;
        check(radius > 0., "パスの半径は正数です")?;
        r.unit("hardness")?;
        r.float("spacing", 0.01, 4.)?;
        r.unit("opacity")?;
        r.unit("flow")?;
        r.blob("rgba", 4)?;
        for k in [
            "erase",
            "pressure_size",
            "pressure_opacity",
            "pressure_flow",
        ] {
            r.boolean(k)?;
        }
        if v >= ANTI_ALIAS_VERSION {
            let level = r.byte("anti_alias")?;
            check(level <= 3, "パスのブラシのアンチエイリアスの段が不正です")?;
        }
        Ok(())
    })?;
    let n = r.int("point_count", 0, 4096)?;
    for i in 0..n {
        r.block(&format!("points[{i}]"), |r| {
            if surface {
                r.int("triangle", 0, i32::MAX)?;
                let u = r.float("u", -1e-9, 1. + 1e-9)?;
                let vv = r.float("v", -1e-9, 1. + 1e-9)?;
                check(u + vv <= 1. + 1e-9, "パスの点が三角形の外です")?;
            } else {
                r.float("x", -1e6, 1e6)?;
                r.float("y", -1e6, 1e6)?;
            }
            r.unit("pressure")?;
            Ok(())
        })?;
    }
    let count = if v >= 18 {
        r.byte("material_count")?
    } else {
        0
    };
    check(count <= 6, "パスのマテリアル数が不正です")?;
    if count == 0 {
        check(
            enabled.contains(&channel),
            "パスのチャンネルが有効ではありません",
        )?;
    }
    let mut seen = HashSet::new();
    for i in 0..count {
        r.block(&format!("material[{i}]"), |r| {
            let c = unique_channel(r, &mut seen)?;
            check(
                channels.contains(&c),
                "パスのマテリアルのチャンネルがありません",
            )?;
            r.blob("rgba", 4)?;
            Ok(())
        })?;
    }
    Ok(n)
}

/// レイヤーのパスの一覧（版 27）: 1〜256 本。1 本ごとに名前（128 文字（UTF-16）まで、制御文字なし）・表示・側（3D か）と、1 本のパスと
/// 同じ並びのパス。側・基準のチャンネル・指紋・ID が揃うかは core が確かめる。
fn path_list(
    r: &mut Reader<'_>,
    v: i32,
    channels: &HashSet<i32>,
    enabled: &HashSet<i32>,
) -> Result<()> {
    let n = r.int("count", 1, 256)?;
    for i in 0..n {
        r.block(&format!("items[{i}]"), |r| {
            let name = r.string("name")?;
            check(
                name.encode_utf16().count() <= 128 && !name.chars().any(char::is_control),
                "パスの名前が不正です",
            )?;
            r.boolean("visible")?;
            let surface = r.boolean("surface")?;
            let points = r.block("path", |r| path(r, v, surface, channels, enabled))?;
            r.block("extra", |r| path_extra(r, v, surface, points))
        })?;
    }
    Ok(())
}

/// 一覧の 1 本の、1 本のパスの並びに無い設定（版 27）: 種類（リボンの画像・並べ方・間隔、指先の強さ）、筆先の画像・角度・
/// 向き、投影の深さ、対称と、角・取っ手の点（滑らかでない点だけ、番号の増える順）。
fn path_extra(r: &mut Reader<'_>, v: i32, surface: bool, points: i32) -> Result<()> {
    let kind = r.byte("kind")?;
    check(kind <= 4, "パスの種類が不正です")?;
    match kind {
        1 => r.block("ribbon", |r| {
            r.id("image", false)?;
            let mode = r.byte("mode")?;
            check(mode <= 1, "リボンの並べ方が不正です")?;
            r.float("spacing", 0.1, 4.0)?;
            Ok(())
        })?,
        3 => {
            r.unit("strength")?;
        }
        _ => {}
    }
    if r.boolean("has_tip")? {
        r.block("tip", |r| {
            let name = r.string("name")?;
            check(name.len() <= 4096, "筆先の名前が長すぎます")?;
            let w = r.int("width", 1, 2048)?;
            let h = r.int("height", 1, 2048)?;
            r.blob("alpha", (w * h) as usize)?;
            Ok(())
        })?;
    }
    r.float("angle", -360.0, 360.0)?;
    r.boolean("follow")?;
    if r.boolean("has_depth")? {
        r.float("depth", 0.05, 64.0)?;
    }
    let symmetry = r.byte("symmetry")?;
    check(
        symmetry == 0 || (symmetry == 1 && !surface) || (symmetry == 2 && surface),
        "パスの対称の種類が不正です",
    )?;
    if symmetry == 1 {
        r.block("canvas_symmetry", |r| {
            // 種類 5（線対称）は正本の版 35 から。線の本数は偶数で、続けて最初の線の角度（度）
            let mode = r.int("mode", 1, if v >= RULERS_VERSION { 5 } else { 4 })?;
            r.float("center_x", -1e7, 1e7)?;
            r.float("center_y", -1e7, 1e7)?;
            let count = r.int("count", 2, 16)?;
            if mode == 5 {
                check(count % 2 == 0, "線対称の線の本数が偶数ではありません")?;
                r.float("angle", -360.0, 360.0)?;
            }
            Ok(())
        })?;
    }
    if symmetry == 2 {
        r.block("mirror", |r| {
            for k in [
                "point_x", "point_y", "point_z", "normal_x", "normal_y", "normal_z",
            ] {
                r.float(k, -1e6, 1e6)?;
            }
            Ok(())
        })?;
    }
    let n = r.int("tangent_count", 0, points)?;
    let mut last = -1;
    for i in 0..n {
        r.block(&format!("tangents[{i}]"), |r| {
            let index = r.int("index", 0, points - 1)?;
            check(index > last, "接線の点の番号が増える順ではありません")?;
            last = index;
            let kind = r.byte("kind")?;
            check(kind == 1 || kind == 2, "接線の種類が不正です")?;
            if kind == 2 {
                let axes: &[&str] = if surface {
                    &["x", "y", "z"]
                } else {
                    &["x", "y"]
                };
                for side in ["incoming", "outgoing"] {
                    r.block(side, |r| {
                        for a in axes {
                            r.float(a, -1e6, 1e6)?;
                        }
                        Ok(())
                    })?;
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn validate_fields(fields: &[NativeField]) -> Result<()> {
    let map: HashMap<&str, &NativeValue> =
        fields.iter().map(|f| (f.path.as_str(), &f.value)).collect();
    let mut filter_ids = HashSet::new();
    for field in fields {
        if field.path.contains(".filters.items[") && field.path.ends_with(".id") {
            if let NativeValue::Guid(id) = field.value {
                check(
                    filter_ids.insert(id),
                    "プロジェクト内のフィルターIDが重複しています",
                )?;
            }
        }
        if field.path.contains(".gradients[") && field.path.ends_with(".type") {
            let prefix = field.path.strip_suffix(".type").unwrap();
            check(
                map.get(format!("{prefix}.algorithm").as_str()) == Some(&&NativeValue::Int(2))
                    && map.get(format!("{prefix}.blend").as_str()) == Some(&&NativeValue::Int(1)),
                "塗りつぶしグラデーションはランプ付き・Replaceが必要です",
            )?;
        }
        if field.path.ends_with(".filters.count") && !field.path.contains(".mask.") {
            let layer = field.path.strip_suffix(".filters.count").unwrap();
            let kind = map.get(format!("{layer}.kind").as_str());
            check(
                field.value == NativeValue::Int(0)
                    || !matches!(kind, Some(NativeValue::Int(2 | 3))),
                "調整・グループには内容フィルターを設定できません",
            )?;
        }
    }
    Ok(())
}
