//! 文書のレイヤーを、GPU の合成が読む平らな命令の並び（レイヤーごとの設定と、GPU に上げる面の番号）に直す。
//! `yolu-core` の CPU の合成（`composite.rs` の計画）と同じ規則で、描くレイヤー・落とすレイヤー・クリッピングの組・グループの入れ子を決める。
//!
//! 命令の並びは、画素ごとに先頭から 1 回だけ流す（再帰しない）。タイルごとには、そのタイルに画素の無いレイヤーの命令を落とした列
//! （[`Plan::tile_program`]）を流す。結果と、クリッピングの下地（組の値）の 2 つのレジスタと、小さな退避の積み（[`STACK_SLOTS`] 語）を持つ:
//! - `LAYER`: レイヤー 1 枚（クリッピングの組を持たない）を結果へ重ねる。
//! - `LOAD`・`CLIP`・`BLEND`: 下地を組の値へ読み、クリッピングのレイヤーを組へ重ね、組を結果へ重ねる。
//! - `PUSH_ISO`・`POP_BASE`・`POP_CLIP`: 独立して合成するグループ（中身を透明から合成する）。結果と組の値を退避して中身を流し、戻ったとき
//!   下地の値（`POP_BASE`）、またはクリッピングされたレイヤーの値（`POP_CLIP`）にする。
//! - `PUSH_PASS`・`POP_PASS`: 不透明度が 1 でない・マスクが効く通過のグループ。中身を下の結果の上へ流し、下とフェードする。
//!   不透明度 1・マスクが効かない通過のグループは、中身をそのまま下へ重ねるのと同じなので、平らにして命令を足さない。
//! - `ADJUST`・`CLIP_ADJUST`: 調整レイヤー（結果、またはクリッピングの組の値を読んで変える）。
//!
//! 作れない文書（文書にないチャンネル・退避の積みに収まらない深さのグループ）は [`Unsupported`] で断る。断ったあとの CPU の合成は
//! 呼び手の仕事で、ここは何も変えない。
use super::LayerData;
use std::{collections::HashMap, fmt};
use yolu_core::{
    filter::Settings as FilterSettings, AdjustmentSettings, AdjustmentType, BalanceRange,
    BlendMode, Channel, ChannelKind, Document, EffectSettings, FilterEffect, Layer, LayerId,
    LayerKind, Rgba8,
};

/// 値のない番号（面を持たない・マスクがない）。シェーダーの `NONE` と同じ。
pub(crate) const NONE: u32 = u32::MAX;

/// グループの入れ子の退避の積みの語数の上限（結果と組の値の退避が独立して合成するグループで 2 語、通過のグループは 1 語）。独立して
/// 合成するグループなら 32 段まで入る。文書の入れ子の上限（`MAX_GROUP_DEPTH` = 64）より浅いので、超える文書は CPU で合成する。
pub(crate) const STACK_SLOTS: u32 = 64;

/// シェーダーの積みの大きさの段階（0 は積みを持たない）。計画が要る深さ以上の最小の段階でシェーダーを作る。
const STACK_CLASSES: [u32; 5] = [0, 8, 16, 32, STACK_SLOTS];

/// 計画が要るシェーダーの形。積み（グループ）・調整の式・法線の式は、使わない文書のシェーダーには入れない: 使わない場合も入れると、
/// 画素ごとの私的な積みと調整の式の分だけループの状態が増え、グループも調整も無い文書の合成が遅くなる（ソフトウェアの GPU の計測で
/// 3 倍）。要る形のシェーダーは、初めて要るときに作る。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Variant {
    /// 積みの語数（[`STACK_CLASSES`] の段階。0 は積みを持たない）。
    pub stack: u32,
    /// 調整の式を持つか。
    pub adjust: bool,
    /// 法線の種類のチャンネルの式（重ねる式が単位ベクトルの式）か。色の式でないときは持たない。
    pub normal: bool,
}

impl Variant {
    /// グループも調整も無い文書の形（作るときに必ず作る）。
    pub(crate) const FLAT: Variant = Variant {
        stack: 0,
        adjust: false,
        normal: false,
    };
}

/// 命令の種類（シェーダーの `OP_*` と同じ。シェーダーの先頭に [`shader_source`] が値を書き足す）。
pub(crate) mod op {
    pub const LAYER: u32 = 0;
    pub const LOAD: u32 = 1;
    pub const CLIP: u32 = 2;
    pub const BLEND: u32 = 3;
    pub const PUSH_ISO: u32 = 4;
    pub const POP_BASE: u32 = 5;
    pub const POP_CLIP: u32 = 6;
    pub const PUSH_PASS: u32 = 7;
    pub const POP_PASS: u32 = 8;
    pub const ADJUST: u32 = 9;
    pub const CLIP_ADJUST: u32 = 10;
}

/// 調整の式の種類（シェーダーの `ADJ_*` と同じ。調整の命令では `slot` に置く）。
pub(crate) mod adj {
    pub const INVERT: u32 = 0;
    /// 成分ごとの 256 の表（レベル補正・トーンカーブ・明るさ/コントラスト）。
    pub const LUT3: u32 = 1;
    pub const HUE_SATURATION: u32 = 2;
    /// 輝度 → 色の 256 の表（グラデーションマップ）。
    pub const GRADIENT: u32 = 3;
    pub const BALANCE: u32 = 4;
    pub const THRESHOLD: u32 = 5;
    pub const POSTERIZE: u32 = 6;
}

/// 1 つの成分の表（256 バイトを 4 つずつ 1 語へ詰めた 64 語）。
const LUT_WORDS: usize = 64;
/// 成分ごとの表の語数（R・G・B の 3 つ）。
const LUT3_WORDS: usize = 3 * LUT_WORDS;
/// グラデーションマップの表の語数（256 の輝度ごとに色を 1 語）。
const GRADIENT_WORDS: usize = 256;

/// シェーダーの全文（定数を Rust の値から書き足す。命令の番号・調整の種類・積みの大きさは Rust とシェーダーで同じ値を使う）。
/// `variant` が持たない部分（積み・調整の式）は、シェーダーの `// @begin 名前` から `// @end 名前` までの区間を落とす。
pub(crate) fn shader_source(variant: Variant) -> String {
    let constants = [
        ("OP_LAYER", op::LAYER),
        ("OP_LOAD", op::LOAD),
        ("OP_CLIP", op::CLIP),
        ("OP_BLEND", op::BLEND),
        ("OP_PUSH_ISO", op::PUSH_ISO),
        ("OP_POP_BASE", op::POP_BASE),
        ("OP_POP_CLIP", op::POP_CLIP),
        ("OP_PUSH_PASS", op::PUSH_PASS),
        ("OP_POP_PASS", op::POP_PASS),
        ("OP_ADJUST", op::ADJUST),
        ("OP_CLIP_ADJUST", op::CLIP_ADJUST),
        ("ADJ_INVERT", adj::INVERT),
        ("ADJ_LUT3", adj::LUT3),
        ("ADJ_HUE_SATURATION", adj::HUE_SATURATION),
        ("ADJ_GRADIENT", adj::GRADIENT),
        ("ADJ_BALANCE", adj::BALANCE),
        ("ADJ_THRESHOLD", adj::THRESHOLD),
        ("ADJ_POSTERIZE", adj::POSTERIZE),
        ("LUT_WORDS", LUT_WORDS as u32),
        ("STACK_SLOTS", variant.stack),
        ("BLEND_OVERLAY", BlendMode::Overlay as u32),
    ];
    let mut source = String::new();
    for (name, value) in constants {
        source.push_str(&format!("const {name}: u32 = {value}u;\n"));
    }
    source.push_str(&strip(include_str!("paint.wgsl"), variant));
    source
}

/// `// @begin 名前` から `// @end 名前` までの区間を、`variant` が持たなければ落とす（区間は入れ子にしない）。
fn strip(text: &str, variant: Variant) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dropping = false;
    for line in text.lines() {
        let t = line.trim();
        if let Some(name) = t.strip_prefix("// @begin ") {
            let keep = match name {
                "stack" => variant.stack > 0,
                "adjust" => variant.adjust,
                "normal" => variant.normal,
                "color" => !variant.normal,
                other => panic!("知らないシェーダーの区間: {other}"),
            };
            dropping = !keep;
        } else if t.starts_with("// @end ") {
            dropping = false;
        } else if !dropping {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// GPU の合成が扱えない理由。扱えない文書は CPU で合成する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// 文書にないチャンネル。
    UnknownChannel,
    /// グループの入れ子が、シェーダーの退避の積みに収まらない深さ（独立して合成するグループで 32 段）。
    GroupDepth,
}

impl Unsupported {
    /// 理由の文（日本語。画面に出す文言は呼び手が種類から作る）。
    pub fn reason(self) -> &'static str {
        match self {
            Unsupported::UnknownChannel => "プロジェクトにないチャンネル",
            Unsupported::GroupDepth => "グループの入れ子が GPU で合成できる深さを超える",
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}
impl std::error::Error for Unsupported {}

/// この文書・チャンネルを GPU で合成できるか。レイヤーの並びだけを見る軽い確認（画素には触れない）。
pub fn supports(doc: &Document, channel: Channel) -> Result<(), Unsupported> {
    Plan::build(doc, channel, false).map(|_| ())
}

/// 上げる面の元。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    /// `doc.layers()` の添字。
    pub layer: usize,
    /// true ならマスクの面（アルファが隠す量）、false ならチャンネルの面。
    pub mask: bool,
    /// 画素が保存した面でなく評価で決まる（有効なフィルター・Generator・塗りつぶしのグラデーションと投影・マスクのフィルター）。
    /// 評価の出力は文書の評価のキャッシュから読む（効果の計算そのものは CPU）。
    pub evaluated: bool,
}

/// GPU の合成の計画: 命令の並び（下から上の順）と、上げる面の並び・調整の表。
pub(crate) struct Plan {
    pub entries: Vec<LayerData>,
    pub slots: Vec<Slot>,
    /// 調整の表と値（調整の命令が `fill` の語の番号で指す。値は f32 のビットで置く）。表を作らない計画（`build` の `tables` が false）では空。
    pub tables: Vec<u32>,
    /// 調整の表・値の語数（表を作らない計画でも数える。GPU の作業域の大きさを決める）。
    pub table_words: usize,
    /// 計画を流すシェーダーの形。
    pub variant: Variant,
}

impl Plan {
    pub(crate) fn metadata(&self) -> Vec<LayerData> {
        self.entries.clone()
    }

    /// 1 つのタイルが流す命令の番号を `out` へ足す（命令の並びから、そのタイルに画素の無いレイヤーを落としたもの）。`present(面の番号)` は、
    /// そのタイルにその面（レイヤー・マスク）の画素があるか。
    ///
    /// 画素の無いレイヤーは何も変えないので、その命令を落としても結果は同じ: 下地が無ければクリッピングの組ごと、中身に描くレイヤーが 1 つも無い
    /// 独立のグループはグループごと、何も描いていない所への調整は落とす（透明な画素への調整は何も変えない）。通過のグループは、中身が
    /// 何も変えないなら下とフェードしても下のまま。多くのレイヤーが疎に描かれた文書で、タイルごとの命令を描いたレイヤーの数ほどに減らす。
    pub(crate) fn tile_program(&self, present: &dyn Fn(u32) -> bool, out: &mut Vec<u32>) {
        let mut culler = Culler {
            ops: &self.entries,
            present,
            at: 0,
        };
        culler.stream(out, false);
    }

    /// 計画を作る。`tables` が false なら調整の表を作らない（断る・見積もるだけの軽い確認用。命令の並びと面は同じ）。
    pub(crate) fn build(
        doc: &Document,
        channel: Channel,
        tables: bool,
    ) -> Result<Plan, Unsupported> {
        let kind = doc
            .channel_info(channel)
            .ok_or(Unsupported::UnknownChannel)?
            .kind;
        let layers = doc.layers();
        let mut children: HashMap<Option<LayerId>, Vec<usize>> = HashMap::new();
        for (i, l) in layers.iter().enumerate() {
            children.entry(l.parent()).or_default().push(i);
        }
        let mut builder = Builder {
            layers,
            children,
            channel,
            kind,
            slots: Vec::new(),
            tables: Vec::new(),
            table_words: 0,
            with_tables: tables,
        };
        let entries = builder.level(None)?;
        let (mut depth, mut deepest) = (0u32, 0u32);
        for e in &entries {
            match e.kind {
                op::PUSH_ISO => depth += 2,
                op::PUSH_PASS => depth += 1,
                op::POP_BASE | op::POP_CLIP => depth -= 2,
                op::POP_PASS => depth -= 1,
                _ => {}
            }
            deepest = deepest.max(depth);
        }
        if deepest > STACK_SLOTS {
            return Err(Unsupported::GroupDepth);
        }
        let variant = Variant {
            stack: STACK_CLASSES
                .into_iter()
                .find(|&c| c >= deepest)
                .expect("上限以下"),
            adjust: entries
                .iter()
                .any(|e| matches!(e.kind, op::ADJUST | op::CLIP_ADJUST)),
            normal: kind == ChannelKind::Normal,
        };
        Ok(Plan {
            entries,
            slots: builder.slots,
            tables: builder.tables,
            table_words: builder.table_words,
            variant,
        })
    }
}

/// 命令の並びを文法どおりに読んで、タイルに画素の無い命令を落とす（[`Plan::tile_program`]）。文法:
/// `列 := 項*`、`項 := LAYER | ADJUST | LOAD 組の項* BLEND | PUSH_ISO 列 POP_BASE 組の項* BLEND | PUSH_PASS 列 POP_PASS`、
/// `組の項 := CLIP | CLIP_ADJUST | PUSH_ISO 列 POP_CLIP`。
struct Culler<'a> {
    ops: &'a [LayerData],
    present: &'a dyn Fn(u32) -> bool,
    at: usize,
}

impl Culler<'_> {
    fn has(&self, slot: u32) -> bool {
        slot == NONE || (self.present)(slot)
    }

    /// 終わり（POP_*）か並びの終わりまで読む。`drew` は、この列より前に何かを描いたか（透明な所への調整を落とす）。
    /// 落とさず出した命令の番号を `out` へ足し、この列が何かを描いた（または描いた状態で終わる）かを返す。
    fn stream(&mut self, out: &mut Vec<u32>, mut drew: bool) -> bool {
        while let Some(op) = self.ops.get(self.at) {
            match op.kind {
                op::POP_BASE | op::POP_CLIP | op::POP_PASS => return drew,
                op::LAYER => {
                    if self.has(op.slot) {
                        out.push(self.at as u32);
                        drew = true;
                    }
                    self.at += 1;
                }
                op::ADJUST => {
                    if drew {
                        out.push(self.at as u32);
                    }
                    self.at += 1;
                }
                op::LOAD => {
                    drew |= self.clipped(out);
                }
                op::PUSH_ISO => {
                    drew |= self.isolated(out);
                }
                op::PUSH_PASS => {
                    let start = out.len();
                    let push = self.at;
                    self.at += 1;
                    let mut inner = Vec::new();
                    let inner_drew = self.stream(&mut inner, drew);
                    // POP_PASS
                    let pop = self.at;
                    self.at += 1;
                    if !inner.is_empty() {
                        out.push(push as u32);
                        out.extend(inner);
                        out.push(pop as u32);
                        drew |= inner_drew;
                    } else {
                        debug_assert_eq!(out.len(), start);
                    }
                }
                other => unreachable!("列の途中に来ない命令: {other}"),
            }
        }
        drew
    }

    /// `LOAD 組の項* BLEND`。下地が無ければ組ごと落とす。何かを描いたかを返す。
    fn clipped(&mut self, out: &mut Vec<u32>) -> bool {
        let load = self.at;
        self.at += 1;
        let drawn = self.has(self.ops[load].slot);
        let mut body = Vec::new();
        self.clips(&mut body, drawn);
        let blend = self.at;
        self.at += 1; // BLEND
        if drawn {
            out.push(load as u32);
            out.extend(body);
            out.push(blend as u32);
        }
        drawn
    }

    /// `PUSH_ISO 列 POP_BASE 組の項* BLEND`。中身に描くレイヤーが 1 つも無ければグループごと落とす。何かを描いたかを返す。
    fn isolated(&mut self, out: &mut Vec<u32>) -> bool {
        let push = self.at;
        self.at += 1;
        let mut inner = Vec::new();
        let drawn = self.stream(&mut inner, false);
        let pop = self.at;
        self.at += 1; // POP_BASE
        let mut body = Vec::new();
        self.clips(&mut body, drawn);
        let blend = self.at;
        self.at += 1; // BLEND
        if drawn {
            out.push(push as u32);
            out.extend(inner);
            out.push(pop as u32);
            out.extend(body);
            out.push(blend as u32);
        }
        drawn
    }

    /// 組の項（クリッピングのレイヤー・調整・グループ）を BLEND まで読む。`base` は組の下地が描かれているか（無ければ全部落とす）。
    fn clips(&mut self, out: &mut Vec<u32>, base: bool) {
        while let Some(op) = self.ops.get(self.at) {
            match op.kind {
                op::BLEND => return,
                op::CLIP => {
                    if base && self.has(op.slot) {
                        out.push(self.at as u32);
                    }
                    self.at += 1;
                }
                op::CLIP_ADJUST => {
                    if base {
                        out.push(self.at as u32);
                    }
                    self.at += 1;
                }
                op::PUSH_ISO => {
                    let push = self.at;
                    self.at += 1;
                    let mut inner = Vec::new();
                    let drawn = self.stream(&mut inner, false);
                    let pop = self.at;
                    self.at += 1; // POP_CLIP
                    if base && drawn {
                        out.push(push as u32);
                        out.extend(inner);
                        out.push(pop as u32);
                    }
                }
                other => unreachable!("組の途中に来ない命令: {other}"),
            }
        }
    }
}

fn instruction(kind: u32, data: &LayerData) -> LayerData {
    LayerData { kind, ..*data }
}

fn marker(kind: u32) -> LayerData {
    LayerData {
        kind,
        ..bytemuck::Zeroable::zeroed()
    }
}

struct Builder<'a> {
    layers: &'a [Layer],
    children: HashMap<Option<LayerId>, Vec<usize>>,
    channel: Channel,
    kind: ChannelKind,
    slots: Vec<Slot>,
    tables: Vec<u32>,
    /// 表の語数（`with_tables` が false でも数える）。
    table_words: usize,
    with_tables: bool,
}

impl Builder<'_> {
    /// そのレイヤーがこのチャンネルで何かを出せるか（CPU の `Stack::active`）。グループは対象外。
    fn active(&self, l: &Layer) -> bool {
        let content = match l.kind() {
            LayerKind::Raster => l.surface(self.channel).is_some(),
            LayerKind::Fill => l.fill_value(self.channel).is_some(),
            LayerKind::Adjustment => l.adjustment().is_some_and(|a| a.applies_to(self.kind)),
            LayerKind::Group => false,
        };
        l.visible()
            && l.opacity_in(self.channel) > 0.0
            && l.is_channel_enabled(self.channel)
            && content
    }

    /// 1 つの段（親の子）の命令を、下から上の順に。CPU の `plan_level` と同じ規則で、兄弟の中で一番下でなくクリッピングの印のあるレイヤーは
    /// すぐ下の下地の組に入る（下地が落ちれば一緒に落ちる）。調整レイヤーは下地にならない（その上のクリッピングは描かない）。
    fn level(&mut self, parent: Option<LayerId>) -> Result<Vec<LayerData>, Unsupported> {
        let layers = self.layers;
        let mut out = Vec::new();
        let Some(siblings) = self.children.get(&parent).cloned() else {
            return Ok(out);
        };
        let mut k = 0;
        while k < siblings.len() {
            // 一番下の兄弟は印があっても下地。そのあとに続く印のあるレイヤーが、この下地の組に入る。
            let mut end = k + 1;
            while end < siblings.len() && layers[siblings[end]].clipping() {
                end += 1;
            }
            let base = siblings[k];
            let clips = &siblings[k + 1..end];
            k = end;
            let l = &layers[base];
            match l.kind() {
                LayerKind::Group => {
                    if !l.visible() || l.opacity_in(self.channel) <= 0.0 {
                        continue;
                    }
                    let inner = self.level(Some(l.id()))?;
                    if inner.is_empty() {
                        continue; // 中身の無いグループは落ちる（クリッピングのレイヤーも一緒に）
                    }
                    let clip_ops = self.clip_instructions(clips)?;
                    let data = self.data(base);
                    if l.blend_mode_in(self.channel) == BlendMode::PassThrough
                        && clip_ops.is_empty()
                    {
                        // 不透明度 1・マスクが効かない通過は、中身をそのまま下へ重ねるのと同じ。クリッピングの組に入るのは描かれるレイヤーだけ
                        // （core の `plan_level` は `make_entry` が None のレイヤーを `clips` に入れない）なので、見えない・不透明度 0 のレイヤーが
                        // 上に並んでいるだけでは、組を持たない通過のままにする。
                        if l.opacity_in(self.channel) == 1.0
                            && l.mask().is_none_or(|m| m.is_neutral())
                        {
                            out.extend(inner);
                        } else {
                            out.push(marker(op::PUSH_PASS));
                            out.extend(inner);
                            out.push(instruction(op::POP_PASS, &data));
                        }
                    } else {
                        out.push(marker(op::PUSH_ISO));
                        out.extend(inner);
                        out.push(marker(op::POP_BASE));
                        out.extend(clip_ops);
                        out.push(instruction(op::BLEND, &data));
                    }
                }
                LayerKind::Adjustment => {
                    if self.active(l) {
                        out.push(self.adjustment(base, op::ADJUST));
                    }
                }
                LayerKind::Raster | LayerKind::Fill => {
                    if !self.active(l) {
                        continue; // 下地が落ちれば、クリッピングのレイヤーも一緒に落ちる
                    }
                    let data = self.data(base);
                    let clip_ops = self.clip_instructions(clips)?;
                    if clip_ops.is_empty() {
                        out.push(instruction(op::LAYER, &data));
                    } else {
                        out.push(instruction(op::LOAD, &data));
                        out.extend(clip_ops);
                        out.push(instruction(op::BLEND, &data));
                    }
                }
            }
        }
        Ok(out)
    }

    /// 下地の組に入るクリッピングのレイヤーの命令（描かれるレイヤーだけ。隠す・不透明度 0・中身が無いレイヤーは入れない）。
    fn clip_instructions(&mut self, clips: &[usize]) -> Result<Vec<LayerData>, Unsupported> {
        let layers = self.layers;
        let mut out = Vec::new();
        for &c in clips {
            let cl = &layers[c];
            match cl.kind() {
                LayerKind::Group => {
                    if !cl.visible() || cl.opacity_in(self.channel) <= 0.0 {
                        continue;
                    }
                    // クリッピングされたグループは通過の指定でも、中身を透明から合成して下地へ重ねる
                    let inner = self.level(Some(cl.id()))?;
                    if inner.is_empty() {
                        continue;
                    }
                    let data = self.data(c);
                    out.push(marker(op::PUSH_ISO));
                    out.extend(inner);
                    out.push(instruction(op::POP_CLIP, &data));
                }
                LayerKind::Adjustment => {
                    if self.active(cl) {
                        out.push(self.adjustment(c, op::CLIP_ADJUST));
                    }
                }
                LayerKind::Raster | LayerKind::Fill => {
                    if self.active(cl) {
                        let data = self.data(c);
                        out.push(instruction(op::CLIP, &data));
                    }
                }
            }
        }
        Ok(out)
    }

    /// レイヤーの設定（不透明度・合成モード・面の番号・マスク）。面は上げる並びへ足す。PassThrough は Normal として重ねる
    /// （通過のグループがフェードするときは、モードを読まない）。
    fn data(&mut self, index: usize) -> LayerData {
        let l = &self.layers[index];
        let mode = l.blend_mode_in(self.channel);
        let mut data = LayerData {
            opacity: l.opacity_in(self.channel) as f32,
            mode: if mode == BlendMode::PassThrough {
                BlendMode::Normal as u32
            } else {
                mode as u32
            },
            slot: NONE,
            mask: NONE,
            ..bytemuck::Zeroable::zeroed()
        };
        let evaluated = l.has_evaluated_output(self.channel);
        match l.kind() {
            LayerKind::Fill if !evaluated => {
                let c = l.fill_value(self.channel).unwrap_or(Rgba8::TRANSPARENT);
                data.fill = u32::from_le_bytes(c.to_array());
            }
            LayerKind::Raster | LayerKind::Fill => {
                data.slot = self.slots.len() as u32;
                self.slots.push(Slot {
                    layer: index,
                    mask: false,
                    evaluated,
                });
            }
            _ => {}
        }
        if let Some(m) = l.mask().filter(|m| !m.is_neutral()) {
            data.mask = self.slots.len() as u32;
            data.mask_invert = u32::from(m.inverted());
            data.mask_density = m.density() as f32;
            self.slots.push(Slot {
                layer: index,
                mask: true,
                evaluated: m.has_active_filters(),
            });
        }
        data
    }

    /// 調整レイヤーの命令。式の種類は `slot`、値・表の語の番号は `fill` に置く（調整の命令は面も塗りつぶしの色も持たない）。
    fn adjustment(&mut self, index: usize, kind: u32) -> LayerData {
        let mut data = self.data(index);
        data.kind = kind;
        let settings = self.layers[index]
            .adjustment()
            .expect("描く調整レイヤーは設定を持つ");
        let (adj, block) = self.encode_adjustment(settings);
        data.slot = adj;
        data.fill = self.table_words as u32;
        self.table_words += block.words;
        if self.with_tables {
            self.tables.extend(block.data);
        }
        data
    }

    /// 調整の式の種類と、その値・表（`tables` の中の連続した語）。`with_tables` が false なら、語数だけ正しい空の値（表の中身は作らない）。
    fn encode_adjustment(&self, a: &AdjustmentSettings) -> (u32, Block) {
        let with = self.with_tables;
        let floats = |values: &[f32]| Block {
            words: values.len(),
            data: values.iter().map(|v| v.to_bits()).collect(),
        };
        match a.kind() {
            AdjustmentType::Invert => (adj::INVERT, Block::empty(0)),
            // 成分ごとに決まる式は、灰色の画素 v を通した結果を表にする。成分が独立なので、R・G・B の表が灰色 1 本の探りで全部引ける
            // （式の中身を繰り返さず、CPU と同じバイトになる）。
            AdjustmentType::Levels
            | AdjustmentType::ToneCurve
            | AdjustmentType::BrightnessContrast => {
                if !with {
                    return (adj::LUT3, Block::empty(LUT3_WORDS));
                }
                let mut words = vec![0u32; LUT3_WORDS];
                for v in 0..=255u8 {
                    let out = a.apply_in(self.kind, Rgba8::new(v, v, v, 255));
                    for (c, byte) in [out.r, out.g, out.b].into_iter().enumerate() {
                        words[c * LUT_WORDS + usize::from(v) / 4] |=
                            u32::from(byte) << ((usize::from(v) % 4) * 8);
                    }
                }
                (
                    adj::LUT3,
                    Block {
                        words: LUT3_WORDS,
                        data: words,
                    },
                )
            }
            AdjustmentType::HueSaturation => (
                adj::HUE_SATURATION,
                floats(&[
                    (a.hue() / 360.0) as f32,
                    a.saturation() as f32,
                    a.lightness() as f32,
                ]),
            ),
            AdjustmentType::GradientMap => {
                if !with {
                    return (adj::GRADIENT, Block::empty(GRADIENT_WORDS));
                }
                let map = a
                    .gradient_map_value()
                    .expect("グラデーションマップは値を持つ");
                // `GradientMap::new` の表と同じ式（輝度 l → ランプの位置 l / 255、逆向きなら (255 − l) / 255）
                let data = (0..=255u32)
                    .map(|l| {
                        let t = if map.reverse() { 255 - l } else { l };
                        let c = map
                            .ramp()
                            .evaluate(f64::from(t) / 255.0, false)
                            .unwrap_or(Rgba8::TRANSPARENT);
                        u32::from_le_bytes(c.to_array())
                    })
                    .collect();
                (
                    adj::GRADIENT,
                    Block {
                        words: GRADIENT_WORDS,
                        data,
                    },
                )
            }
            AdjustmentType::ColorBalance => {
                let b = a.color_balance_value().expect("カラーバランスは値を持つ");
                let mut v = [0f32; 11];
                for (r, range) in [
                    BalanceRange::Shadows,
                    BalanceRange::Midtones,
                    BalanceRange::Highlights,
                ]
                .into_iter()
                .enumerate()
                {
                    for (k, x) in b.values(range).into_iter().enumerate() {
                        v[r * 3 + k] = x as f32;
                    }
                }
                v[9] = f32::from(u8::from(b.preserve_luminosity()));
                v[10] = f32::from(u8::from(b.is_neutral()));
                (adj::BALANCE, floats(&v))
            }
            AdjustmentType::Threshold => (
                adj::THRESHOLD,
                Block {
                    words: 1,
                    data: vec![a.threshold_value().map_or(0, |t| t.level())],
                },
            ),
            AdjustmentType::Posterize => (
                adj::POSTERIZE,
                Block {
                    words: 1,
                    data: vec![a.posterize_value().map_or(2, |t| t.levels())],
                },
            ),
        }
    }
}

/// 調整の命令が指す、`tables` の中の連続した語。
struct Block {
    words: usize,
    data: Vec<u32>,
}
impl Block {
    fn empty(words: usize) -> Block {
        Block {
            words,
            data: Vec::new(),
        }
    }
}

/// 評価の出力を持つ面が、全部を常駐させたとき持ち得るタイルの数の上限（CPU の評価の `may_cover` と同じ考え方の、公開の口で組める見積もり）。
/// 塗りつぶし・Generator・画像の投影はキャンバス全体、ぼかしは元のタイルを半径の分だけ広げた範囲。見積もりなので、実際に常駐する量
/// （全部 0 の出力は常駐させない）はこれ以下。
pub(crate) fn evaluated_tile_bound(doc: &Document, channel: Channel, slot: &Slot) -> u64 {
    let ts = u64::from(doc.tile_size());
    let total = u64::from(doc.width()).div_ceil(ts) * u64::from(doc.height()).div_ceil(ts);
    let layer = &doc.layers()[slot.layer];
    // (元の面のタイル, 元のタイルの近くだけに出力が出る段だけなら、広げる半径)
    let (source, reach): (u64, Option<u32>) = if slot.mask {
        let Some(mask) = layer.mask() else {
            return 0;
        };
        let chain: Vec<&FilterEffect> = mask.filters().iter().filter(|e| e.is_active()).collect();
        // マスクは、半径の中がすべて 0 の所が 0 のままの段（ぼかし・シャープ・正規化）だけなら、元のタイルの近くだけ
        let preserving = chain.iter().all(|e| {
            matches!(
                e.settings(),
                EffectSettings::Filter(
                    FilterSettings::GaussianBlur { .. }
                        | FilterSettings::Sharpen { .. }
                        | FilterSettings::Normalize
                )
            )
        });
        let halo: u32 = chain.iter().map(|e| e.settings().halo()).sum();
        (
            mask.surface().tile_count() as u64,
            preserving.then_some(halo),
        )
    } else if layer.kind() == LayerKind::Fill {
        (0, None)
    } else {
        let expansion: u32 = layer
            .filters()
            .iter()
            .filter(|e| e.is_active() && e.applies_to(channel))
            .filter(|e| e.settings().expands_coverage())
            .map(|e| e.settings().halo())
            .sum();
        (
            layer.surface(channel).map_or(0, |s| s.tile_count()) as u64,
            Some(expansion),
        )
    };
    match reach {
        Some(halo) => {
            let m = u64::from(halo).div_ceil(ts);
            total.min(source.saturating_mul((2 * m + 1) * (2 * m + 1)))
        }
        None => total,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::{AdjustmentSettings, TileCoord};

    /// タイルの命令の番号を、命令の種類の並びにする（読みやすい確かめのため）。
    fn kinds(plan: &Plan, doc: &Document, coord: TileCoord) -> Vec<u32> {
        let mut out = Vec::new();
        plan.tile_program(
            &|slot| {
                let s = plan.slots[slot as usize];
                let layer = &doc.layers()[s.layer];
                let surface = if s.mask {
                    layer.mask().map(|m| m.surface())
                } else {
                    layer.surface(Channel::Color)
                };
                surface.is_some_and(|f| f.has_tile(coord))
            },
            &mut out,
        );
        out.iter().map(|&k| plan.entries[k as usize].kind).collect()
    }

    fn paint_tile(d: &mut Document, layer: LayerId, coord: TileCoord) {
        let ts = d.tile_size();
        d.set_pixel(
            layer,
            coord.x * ts + 1,
            coord.y * ts + 1,
            Rgba8::new(9, 8, 7, 255),
        )
        .unwrap();
    }

    fn doc() -> Document {
        Document::with_tile_size(64, 64, 32).unwrap()
    }

    /// 積み・調整の式・法線の式の組み合わせのどのシェーダーも、WGSL として読めて検証に通る（GPU の無い環境でも確かめられる）。
    /// 構造体の並びは Rust の `LayerData` と合う。
    #[test]
    fn every_shader_variant_validates_and_matches_the_rust_layout() {
        for stack in STACK_CLASSES {
            for adjust in [false, true] {
                for normal in [false, true] {
                    let variant = Variant {
                        stack,
                        adjust,
                        normal,
                    };
                    let source = shader_source(variant);
                    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| {
                        panic!(
                            "{variant:?}: WGSL を読めない: {}",
                            e.emit_to_string(&source)
                        )
                    });
                    naga::valid::Validator::new(
                        naga::valid::ValidationFlags::all(),
                        naga::valid::Capabilities::empty(),
                    )
                    .validate(&module)
                    .unwrap_or_else(|e| {
                        panic!(
                            "{variant:?}: WGSL の検証に失敗: {}",
                            e.emit_to_string(&source)
                        )
                    });
                    // 命令の構造体は Rust の LayerData と同じ並び（8 語・32 バイト）
                    let layer = module
                        .types
                        .iter()
                        .find(|(_, t)| t.name.as_deref() == Some("Layer"))
                        .map(|(_, t)| t)
                        .expect("Layer がある");
                    let naga::TypeInner::Struct { members, span } = &layer.inner else {
                        panic!("Layer は構造体");
                    };
                    assert_eq!(*span as usize, std::mem::size_of::<LayerData>());
                    let names: Vec<_> =
                        members.iter().map(|m| m.name.as_deref().unwrap()).collect();
                    assert_eq!(
                        names,
                        [
                            "kind",
                            "opacity",
                            "mode",
                            "slot",
                            "mask",
                            "fill",
                            "mask_invert",
                            "mask_density"
                        ]
                    );
                    for (i, m) in members.iter().enumerate() {
                        assert_eq!(m.offset as usize, i * 4, "{:?}", m.name);
                    }
                }
            }
        }
    }

    #[test]
    fn layers_without_pixels_in_a_tile_are_left_out_of_its_program() {
        let mut d = doc();
        let a = d.add_layer("a").unwrap();
        let b = d.add_layer("b").unwrap();
        let c = d.add_layer("c").unwrap();
        paint_tile(&mut d, a, TileCoord::new(0, 0));
        paint_tile(&mut d, b, TileCoord::new(1, 0));
        paint_tile(&mut d, c, TileCoord::new(0, 0));
        paint_tile(&mut d, c, TileCoord::new(1, 0));
        let plan = Plan::build(&d, Channel::Color, false).unwrap();
        assert_eq!(plan.entries.len(), 3);
        assert_eq!(
            kinds(&plan, &d, TileCoord::new(0, 0)),
            [op::LAYER, op::LAYER]
        );
        assert_eq!(
            kinds(&plan, &d, TileCoord::new(1, 0)),
            [op::LAYER, op::LAYER]
        );
        assert_eq!(kinds(&plan, &d, TileCoord::new(1, 1)), Vec::<u32>::new());
        // 無いタイルのレイヤーだけを落とす: 順序は保つ
        let mut out = Vec::new();
        plan.tile_program(&|slot| plan.slots[slot as usize].layer != 1, &mut out);
        assert_eq!(out, [0, 2]);
    }

    #[test]
    fn a_missing_base_drops_the_whole_clipping_group_and_an_empty_isolated_group_is_dropped() {
        let mut d = doc();
        let base = d.add_layer("下地").unwrap();
        let clip = d.add_layer("クリップ").unwrap();
        d.set_layer_clipping(clip, true).unwrap();
        paint_tile(&mut d, clip, TileCoord::new(0, 0));
        paint_tile(&mut d, base, TileCoord::new(1, 0));
        paint_tile(&mut d, clip, TileCoord::new(1, 0));
        let inner = d.add_layer("中").unwrap();
        paint_tile(&mut d, inner, TileCoord::new(1, 0));
        let group = d.group_layers(&[inner], "組").unwrap();
        d.set_layer_blend_mode(group, BlendMode::Multiply).unwrap();
        let plan = Plan::build(&d, Channel::Color, false).unwrap();
        // (0,0): 下地が無いので組ごと落ち、中身の無い独立のグループも落ちる
        assert_eq!(kinds(&plan, &d, TileCoord::new(0, 0)), Vec::<u32>::new());
        // (1,0): 下地とクリッピングのレイヤーの組と、独立のグループ
        assert_eq!(
            kinds(&plan, &d, TileCoord::new(1, 0)),
            [
                op::LOAD,
                op::CLIP,
                op::BLEND,
                op::PUSH_ISO,
                op::LAYER,
                op::POP_BASE,
                op::BLEND
            ]
        );
    }

    #[test]
    fn adjustments_on_nothing_are_dropped_and_pass_groups_follow_their_backdrop() {
        let mut d = doc();
        let a = d.add_layer("a").unwrap();
        paint_tile(&mut d, a, TileCoord::new(0, 0));
        let adj = d
            .add_adjustment_layer("反転", AdjustmentSettings::invert(), None, None)
            .unwrap();
        let b = d.add_layer("b").unwrap();
        paint_tile(&mut d, b, TileCoord::new(1, 1));
        let inner_adj = d
            .add_adjustment_layer("中の反転", AdjustmentSettings::invert(), None, None)
            .unwrap();
        let pass = d.group_layers(&[inner_adj], "通過").unwrap();
        d.set_layer_blend_mode(pass, BlendMode::PassThrough)
            .unwrap();
        d.set_layer_opacity(pass, 0.5, false).unwrap();
        let _ = adj;
        let plan = Plan::build(&d, Channel::Color, false).unwrap();
        // (0,0): a の上の調整と、a を下に持つ通過のグループの中の調整
        assert_eq!(
            kinds(&plan, &d, TileCoord::new(0, 0)),
            [
                op::LAYER,
                op::ADJUST,
                op::PUSH_PASS,
                op::ADJUST,
                op::POP_PASS
            ]
        );
        // (1,1): b より下には何も無いので、a の上の調整は落ち、b の上の通過のグループの調整だけが残る
        assert_eq!(
            kinds(&plan, &d, TileCoord::new(1, 1)),
            [op::LAYER, op::PUSH_PASS, op::ADJUST, op::POP_PASS]
        );
        // (1,0): 何も描いていないので全部落ちる（透明な画素への調整は何も変えない）
        assert_eq!(kinds(&plan, &d, TileCoord::new(1, 0)), Vec::<u32>::new());
    }
}
