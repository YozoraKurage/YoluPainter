//! CPU の合成（C# の CpuCompositor の Plan・EvaluatePixel と、タイルの経路の Node.Load・EvaluateRect・BlendRect・ClipRect）。
//!
//! - 層は下から上へ、透明から始めて 1 段ずつ重ね、段ごとに RGBA8 へ丸める。
//! - 計画（[`plan`]）: 兄弟の中で一番下でなくクリッピングの印のある層は、すぐ下の印の無い兄弟（下地）の組に入る。見えない下地
//!   （非表示・そのチャンネルの不透明度 0・中身が無い）は組ごと落とす。調整の層は下地にならない（その上のクリッピングは描かない）。
//!   グループは中身の計画を持ち、見えないグループ・中身の無いグループは落とす。
//! - 通過のグループ（PassThrough でクリッピングの組を持たない）は中身を下の結果へ重ね、不透明度 × マスクで下とフェードする
//!   （不透明度 1 でマスクが効かなければ、フェードは中身そのものなので下へそのまま重ねる）。ほかのグループは中身を透明から
//!   合成して、層と同じように重ねる（クリッピングされたグループは通過の指定でも透明から）。
//! - マスクの量は層のアルファに掛ける（不透明度 × マスクの値）。何も変えないマスク（無効・濃度 0・空）は掛けない（値はちょうど 1）。
//! - タイルの経路は、そのタイルに何も無い層（グループは中身に画素も調整も無いもの）を飛ばす（C# と同じ）。
//! - Normal の種類のチャンネルは [`crate::normal`] のベクトルの式で、ほかは色の式で重ねる。調整はどちらも色の式。
//! - 画素の値は自分の入力だけで決まるので、どのスレッドがどの行を受け持っても同じバイトになる。

use rayon::prelude::*;

use crate::adjust::AdjustKernel;
use crate::blend::{blend, blend_rgb, clip_onto, fade, separable_table, MIN_SHORTCUT_ALPHA};
use crate::document::EvalSet;
use crate::layer::{Layer, LayerId};
use crate::math::{to_byte, UNIT};
use crate::normal;
use crate::surface::{Surface, Tile};
use crate::types::{BlendMode, Channel, ChannelKind, LayerKind, Rect, Rgba8, RowOrder, TileCoord};

/// 合成の 1 段: 層（またはグループ）と、その組に入るクリッピングされた層（下から上）。グループは中身の計画を持つ。
#[derive(Clone, Debug)]
pub(crate) struct Entry {
    pub layer: usize,
    /// そのチャンネルでの不透明度（[`Layer::opacity_in`]）。
    pub opacity: f64,
    /// そのチャンネルでのモード（[`Layer::blend_mode_in`]）。PassThrough はグループだけ。
    pub mode: BlendMode,
    pub clips: Vec<Entry>,
    pub children: Vec<Entry>,
}

impl Entry {
    /// 重ねるモード（PassThrough は Normal）。
    fn blend_mode(&self) -> BlendMode {
        if self.mode == BlendMode::PassThrough {
            BlendMode::Normal
        } else {
            self.mode
        }
    }
    /// 通過のグループ（クリッピングの組を持たない）: 中身を下へ直に重ねる。
    fn passes_through(&self, layers: &[Layer]) -> bool {
        layers[self.layer].is_group()
            && self.mode == BlendMode::PassThrough
            && self.clips.is_empty()
    }
}

/// 層の並びと、親ごとの子の並び（下から上）。
pub(crate) struct Stack<'a> {
    pub layers: &'a [Layer],
    children: std::collections::HashMap<Option<LayerId>, Vec<usize>>,
    channel: Channel,
    kind: ChannelKind,
    /// 評価した層の出力（フィルター・Generator・画像・グラデーションを通したもの）。あれば、元の画素の代わりに読む。
    eval: Option<&'a EvalSet>,
}

impl<'a> Stack<'a> {
    pub(crate) fn new(
        layers: &'a [Layer],
        channel: Channel,
        kind: ChannelKind,
        eval: Option<&'a EvalSet>,
    ) -> Self {
        let mut children: std::collections::HashMap<Option<LayerId>, Vec<usize>> =
            std::collections::HashMap::new();
        for (i, l) in layers.iter().enumerate() {
            children.entry(l.parent).or_default().push(i);
        }
        Stack {
            layers,
            children,
            channel,
            kind,
            eval,
        }
    }

    /// 層（番号）の評価済みの出力の面（評価が要らない層は None）。
    fn evaluated_content(&self, layer: usize) -> Option<&'a Surface> {
        self.eval.and_then(|e| e.content.get(&layer))
    }
    /// 層のマスクの評価済みの隠す量の面（マスクにフィルターが無ければ None）。
    fn evaluated_mask(&self, layer: usize) -> Option<&'a Surface> {
        self.eval.and_then(|e| e.masks.get(&layer))
    }
    /// 層の画素（マスク・不透明度・合成の前。評価した出力があればそれ）。
    pub(crate) fn layer_pixel(&self, layer: usize, x: u32, y: u32) -> Rgba8 {
        match self.evaluated_content(layer) {
            Some(s) => s.pixel(x, y).unwrap_or(Rgba8::TRANSPARENT),
            None => self.layers[layer].pixel_or_transparent(self.channel, x, y),
        }
    }
    /// 層のマスクが層のアルファに掛ける値。
    pub(crate) fn mask_factor(&self, layer: usize, x: u32, y: u32) -> f64 {
        match &self.layers[layer].mask {
            None => 1.0,
            Some(m) => {
                let surface = self.evaluated_mask(layer).unwrap_or(&m.surface);
                m.factor(surface.pixel(x, y).map_or(0, |p| p.a))
            }
        }
    }

    fn siblings(&self, parent: Option<LayerId>) -> &[usize] {
        self.children.get(&parent).map_or(&[], |v| v.as_slice())
    }

    /// その層がこのチャンネルで何かを出せるか（C# の Active）。
    fn active(&self, layer: &Layer) -> bool {
        let applies = layer
            .adjustment
            .as_ref()
            .is_some_and(|a| a.applies_to(self.kind));
        layer.visible
            && layer.opacity_in(self.channel) > 0.0
            && layer.is_channel_enabled(self.channel)
            && layer.has_content(self.channel, applies)
    }

    pub(crate) fn make_entry(&self, i: usize) -> Option<Entry> {
        let l = &self.layers[i];
        if l.is_group() {
            if !l.visible || l.opacity_in(self.channel) <= 0.0 {
                return None;
            }
            let children = self.plan_level(Some(l.id));
            if children.is_empty() {
                return None;
            }
            return Some(Entry {
                layer: i,
                opacity: l.opacity_in(self.channel),
                mode: l.blend_mode_in(self.channel),
                clips: Vec::new(),
                children,
            });
        }
        self.active(l).then(|| Entry {
            layer: i,
            opacity: l.opacity_in(self.channel),
            mode: l.blend_mode_in(self.channel),
            clips: Vec::new(),
            children: Vec::new(),
        })
    }

    /// 1 つの段（親の子）の計画（C# の PlanLevel）。
    pub(crate) fn plan_level(&self, parent: Option<LayerId>) -> Vec<Entry> {
        let siblings = self.siblings(parent);
        let mut plan = Vec::new();
        for (k, &i) in siblings.iter().enumerate() {
            if k > 0 && self.layers[i].clipping {
                continue; // 下の下地の組に入る（下地が落ちたなら一緒に落ちる）
            }
            let Some(mut entry) = self.make_entry(i) else {
                continue;
            };
            if self.layers[i].kind != LayerKind::Adjustment {
                let mut m = k + 1;
                while m < siblings.len() && self.layers[siblings[m]].clipping {
                    if let Some(clip) = self.make_entry(siblings[m]) {
                        entry.clips.push(clip);
                    }
                    m += 1;
                }
            }
            plan.push(entry);
        }
        plan
    }

    /// 一番上の段からの計画（C# の Plan）。
    pub(crate) fn plan(&self) -> Vec<Entry> {
        self.plan_level(None)
    }
}

fn count_entries(plan: &[Entry]) -> u64 {
    plan.iter()
        .map(|e| 1 + count_entries(&e.children) + count_entries(&e.clips))
        .sum()
}

fn plan_depth(plan: &[Entry]) -> usize {
    plan.iter()
        .map(|e| {
            let inner = plan_depth(&e.children).max(
                e.clips
                    .iter()
                    .map(|c| plan_depth(&c.children))
                    .max()
                    .unwrap_or(0),
            );
            if e.children.is_empty() && e.clips.iter().all(|c| c.children.is_empty()) {
                0
            } else {
                1 + inner
            }
        })
        .max()
        .unwrap_or(0)
}

// ───────── 画素ごとの参照の式（C# の EvaluatePixel） ─────────

#[inline]
fn stack_blend(normal: bool, below: Rgba8, over: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    if normal {
        normal::blend_unchecked(below, over, amount, mode)
    } else {
        blend(below, over, amount, mode)
    }
}
#[inline]
fn stack_clip(normal: bool, group: Rgba8, clipped: Rgba8, amount: f64, mode: BlendMode) -> Rgba8 {
    if normal {
        normal::clip_onto(group, clipped, amount, mode)
    } else {
        clip_onto(group, clipped, amount, mode)
    }
}
#[inline]
fn stack_fade(normal: bool, backdrop: Rgba8, inner: Rgba8, amount: f64) -> Rgba8 {
    if normal {
        normal::fade(backdrop, inner, amount)
    } else {
        fade(backdrop, inner, amount)
    }
}

pub(crate) fn evaluate_pixel(
    stack: &Stack<'_>,
    plan: &[Entry],
    backdrop: Rgba8,
    x: u32,
    y: u32,
) -> Rgba8 {
    let layers = stack.layers;
    let normal = stack.kind == ChannelKind::Normal;
    let mut result = backdrop;
    for entry in plan {
        let layer = &layers[entry.layer];
        let amount = entry.opacity * stack.mask_factor(entry.layer, x, y);
        if layer.kind == LayerKind::Adjustment {
            let a = layer.adjustment.as_ref().expect("調整の層は設定を持つ");
            result = a.composite_in(stack.kind, result, amount, entry.mode);
            continue;
        }
        if entry.passes_through(layers) {
            let inner = evaluate_pixel(stack, &entry.children, result, x, y);
            result = stack_fade(normal, result, inner, amount);
            continue;
        }
        let mut group = if layer.is_group() {
            evaluate_pixel(stack, &entry.children, Rgba8::TRANSPARENT, x, y)
        } else {
            stack.layer_pixel(entry.layer, x, y)
        };
        for clip in &entry.clips {
            let c = &layers[clip.layer];
            let clip_amount = clip.opacity * stack.mask_factor(clip.layer, x, y);
            if c.kind == LayerKind::Adjustment {
                let a = c.adjustment.as_ref().expect("調整の層は設定を持つ");
                group = a.composite_in(stack.kind, group, clip_amount, clip.mode);
            } else {
                let over = if c.is_group() {
                    evaluate_pixel(stack, &clip.children, Rgba8::TRANSPARENT, x, y)
                } else {
                    stack.layer_pixel(clip.layer, x, y)
                };
                group = stack_clip(normal, group, over, clip_amount, clip.blend_mode());
            }
        }
        result = stack_blend(normal, result, group, amount, entry.blend_mode());
    }
    result
}

/// 画素 1 つの合成（C# の CompositePixel。タイルの経路と照らし合わせる参照の式）。
pub(crate) fn composite_pixel(stack: &Stack<'_>, x: u32, y: u32) -> Rgba8 {
    evaluate_pixel(stack, &stack.plan(), Rgba8::TRANSPARENT, x, y)
}

// ───────── 行の核 ─────────

/// 行の並び: 行 r の最初の画素は bytes[off + r × stride]、画素は step バイトおき（一様なタイルは stride・step とも 0）。
#[derive(Clone, Copy)]
struct Rows<'a> {
    bytes: &'a [u8],
    off: usize,
    stride: usize,
    step: usize,
}

impl<'a> Rows<'a> {
    #[inline]
    fn row(&self, r: usize) -> &'a [u8] {
        &self.bytes[self.off + r * self.stride..]
    }
}

/// 書き先の行の並び: 行 r（下から）は row(r) バイト目から。flip なら上下を逆に置く（上から下の出力へ直に書く）。
#[derive(Clone, Copy)]
struct Out {
    stride: usize,
    flip: Option<usize>,
}

impl Out {
    fn packed(stride: usize) -> Out {
        Out { stride, flip: None }
    }
    #[inline(always)]
    fn row(&self, r: usize) -> usize {
        match self.flip {
            None => r * self.stride,
            Some(last) => (last - r) * self.stride,
        }
    }
}

/// 画素ごとの量: 不透明度、またはマスクがあれば 不透明度 × 表[マスクのアルファ]。
#[derive(Clone, Copy)]
struct Amount<'a> {
    opacity: f64,
    mask: Option<(Rows<'a>, &'a [f64; 256])>,
}

/// 1 行ぶんの量の読み方。
#[derive(Clone, Copy)]
struct RowAmount<'a> {
    opacity: f64,
    mask: Option<(&'a [u8], usize, &'a [f64; 256])>,
}

impl<'a> Amount<'a> {
    #[inline]
    fn row(&self, r: usize) -> RowAmount<'a> {
        RowAmount {
            opacity: self.opacity,
            mask: self.mask.map(|(m, f)| (m.row(r), m.step, f)),
        }
    }
}

impl RowAmount<'_> {
    #[inline(always)]
    fn at(&self, i: usize) -> f64 {
        match self.mask {
            None => self.opacity,
            Some((m, step, f)) => self.opacity * f[m[i * step + 3] as usize],
        }
    }
}

/// 下（res）に上（src）を重ねる（C# の BlendRect の 1 行。各画素は `blend` と同じバイト）。
#[inline]
fn blend_span(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
    table: Option<&[f64]>,
) {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    let count = res.len() / 4;
    for i in 0..count {
        let r = i * 4;
        let s = i * step;
        let s_a = sb[s + 3];
        if s_a == 0 {
            continue; // 上が透明: 下のまま
        }
        let amount = amount.at(i);
        if simple && s_a == 255 && amount == 1.0 {
            res[r..r + 3].copy_from_slice(&sb[s..s + 3]);
            res[r + 3] = 255;
            continue;
        }
        let sa = UNIT[s_a as usize] * amount;
        if sa <= 0.0 {
            continue;
        }
        let d_a = res[r + 3];
        if d_a == 0 && sa >= MIN_SHORTCUT_ALPHA {
            // 下が透明: 重みは 0・a_s・0 で色は (a_s·c)/a_s。積が正規化数なら c から 2 ulp 以内で、丸めると c のバイト
            res[r..r + 3].copy_from_slice(&sb[s..s + 3]);
            res[r + 3] = to_byte(sa);
            continue;
        }
        let (dr, dg, db) = (
            UNIT[res[r] as usize],
            UNIT[res[r + 1] as usize],
            UNIT[res[r + 2] as usize],
        );
        let (sr, sg, sbb) = (
            UNIT[sb[s] as usize],
            UNIT[sb[s + 1] as usize],
            UNIT[sb[s + 2] as usize],
        );
        let (br, bg, bb) = if simple {
            (sr, sg, sbb)
        } else if let Some(t) = table {
            (
                t[(res[r] as usize) << 8 | sb[s] as usize],
                t[(res[r + 1] as usize) << 8 | sb[s + 1] as usize],
                t[(res[r + 2] as usize) << 8 | sb[s + 2] as usize],
            )
        } else {
            blend_rgb(mode, dr, dg, db, sr, sg, sbb)
        };
        let (vr, vg, vb);
        if d_a == 255 {
            // 下が不透明: a = a_s + (1 − a_s) はちょうど 1、重みは 1 − a_s・0・a_s（0 の項と ÷1 は値を変えない）
            let t = 1.0 - sa;
            vr = t * dr + sa * br;
            vg = t * dg + sa * bg;
            vb = t * db + sa * bb;
            res[r + 3] = 255;
        } else {
            let da = UNIT[d_a as usize];
            let a = sa + da * (1.0 - sa);
            let wd = (1.0 - sa) * da;
            let ws = (1.0 - da) * sa;
            let wb = da * sa;
            vr = (wd * dr + ws * sr + wb * br) / a;
            vg = (wd * dg + ws * sg + wb * bg) / a;
            vb = (wd * db + ws * sbb + wb * bb) / a;
            res[r + 3] = to_byte(a);
        }
        res[r] = to_byte(vr);
        res[r + 1] = to_byte(vg);
        res[r + 2] = to_byte(vb);
    }
}

/// クリッピングの下地（g）へクリッピングされた層（c）を重ねる（C# の ClipRect の 1 行。各画素は `clip_onto` と同じバイト）。
#[inline]
fn clip_span(
    g: &mut [u8],
    cb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
    table: Option<&[f64]>,
) {
    let simple = mode == BlendMode::Normal || mode == BlendMode::PassThrough;
    let count = g.len() / 4;
    for i in 0..count {
        let r = i * 4;
        let s = i * step;
        let c_a = cb[s + 3];
        if c_a == 0 || g[r + 3] == 0 {
            continue; // 量 0、または描く下地が無い
        }
        let a = UNIT[c_a as usize] * amount.at(i);
        if a <= 0.0 {
            continue;
        }
        let (dr, dg, db) = (
            UNIT[g[r] as usize],
            UNIT[g[r + 1] as usize],
            UNIT[g[r + 2] as usize],
        );
        let (br, bg, bb) = if simple {
            (
                UNIT[cb[s] as usize],
                UNIT[cb[s + 1] as usize],
                UNIT[cb[s + 2] as usize],
            )
        } else if let Some(t) = table {
            (
                t[(g[r] as usize) << 8 | cb[s] as usize],
                t[(g[r + 1] as usize) << 8 | cb[s + 1] as usize],
                t[(g[r + 2] as usize) << 8 | cb[s + 2] as usize],
            )
        } else {
            blend_rgb(
                mode,
                dr,
                dg,
                db,
                UNIT[cb[s] as usize],
                UNIT[cb[s + 1] as usize],
                UNIT[cb[s + 2] as usize],
            )
        };
        g[r] = to_byte(dr + (br - dr) * a);
        g[r + 1] = to_byte(dg + (bg - dg) * a);
        g[r + 2] = to_byte(db + (bb - db) * a);
    }
}

/// Normal のチャンネルの重ね（画素ごとに [`normal::blend_unchecked`]）。
#[inline]
fn normal_blend_span(
    res: &mut [u8],
    sb: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    mode: BlendMode,
) {
    for i in 0..res.len() / 4 {
        let r = i * 4;
        let v = normal::blend_unchecked(
            Rgba8::from_slice(&res[r..]),
            Rgba8::from_slice(&sb[i * step..]),
            amount.at(i),
            mode,
        );
        res[r..r + 4].copy_from_slice(&v.to_array());
    }
}

/// Normal のチャンネルのクリッピング（画素ごとに [`normal::clip_onto`]）。
#[inline]
fn normal_clip_span(g: &mut [u8], cb: &[u8], step: usize, amount: RowAmount<'_>, mode: BlendMode) {
    for i in 0..g.len() / 4 {
        let r = i * 4;
        let v = normal::clip_onto(
            Rgba8::from_slice(&g[r..]),
            Rgba8::from_slice(&cb[i * step..]),
            amount.at(i),
            mode,
        );
        g[r..r + 4].copy_from_slice(&v.to_array());
    }
}

// ───────── タイルの経路 ─────────

/// 計画の 1 段に解いた材料。
enum Content<'a> {
    Raster(&'a Surface),
    /// 塗りつぶしの値（無い・透明なら None: そのチャンネルに画素は無い）。
    Fill(Option<Rgba8>),
    Adjust(AdjustKernel),
    Group,
}

struct Node<'a> {
    content: Content<'a>,
    opacity: f64,
    /// 重ねるモード（PassThrough は Normal）。
    mode: BlendMode,
    /// 計画のモードそのもの（調整の合成に使う）。
    raw_mode: BlendMode,
    table: Option<&'static [f64]>,
    /// 何かを変えるマスクだけ（面と、隠す量の表）。
    mask: Option<(&'a Surface, Box<[f64; 256]>)>,
    passes_through: bool,
    children: Vec<usize>,
    clips: Vec<usize>,
}

struct Plan<'a> {
    nodes: Vec<Node<'a>>,
    roots: Vec<usize>,
    normal: bool,
    tile_size: usize,
}

impl<'a> Plan<'a> {
    fn build(stack: &Stack<'a>, plan: &[Entry]) -> Plan<'a> {
        let mut p = Plan {
            nodes: Vec::new(),
            roots: Vec::new(),
            normal: stack.kind == ChannelKind::Normal,
            tile_size: 0,
        };
        p.roots = plan.iter().map(|e| p.add(stack, e)).collect();
        p
    }

    fn add(&mut self, stack: &Stack<'a>, e: &Entry) -> usize {
        let layers: &'a [Layer] = stack.layers;
        let layer: &'a Layer = &layers[e.layer];
        let content = match layer.kind {
            LayerKind::Raster => Content::Raster(
                stack
                    .evaluated_content(e.layer)
                    .unwrap_or_else(|| layer.surface(stack.channel).expect("計画の層は面を持つ")),
            ),
            LayerKind::Fill => match stack.evaluated_content(e.layer) {
                Some(surface) => Content::Raster(surface),
                None => Content::Fill(
                    layer
                        .fill_value(stack.channel)
                        .filter(|c| *c != Rgba8::TRANSPARENT),
                ),
            },
            LayerKind::Adjustment => Content::Adjust(
                layer
                    .adjustment
                    .as_ref()
                    .expect("調整の層は設定を持つ")
                    .kernel(stack.kind),
            ),
            LayerKind::Group => Content::Group,
        };
        let mode = e.blend_mode();
        let mask = layer.mask.as_ref().filter(|m| !m.is_neutral()).map(|m| {
            (
                stack.evaluated_mask(e.layer).unwrap_or(&m.surface),
                Box::new(m.factor_table()),
            )
        });
        let id = self.nodes.len();
        self.nodes.push(Node {
            content,
            opacity: e.opacity,
            mode,
            raw_mode: e.mode,
            table: if stack.kind == ChannelKind::Normal {
                None
            } else {
                separable_table(mode)
            },
            mask,
            passes_through: e.passes_through(stack.layers),
            children: Vec::new(),
            clips: Vec::new(),
        });
        let children = e.children.iter().map(|c| self.add(stack, c)).collect();
        let clips = e.clips.iter().map(|c| self.add(stack, c)).collect();
        self.nodes[id].children = children;
        self.nodes[id].clips = clips;
        id
    }
}

/// あるタイルでの層の画素・マスクの読み元。
#[derive(Clone, Copy)]
enum Src<'a> {
    Absent,
    Uniform([u8; 4]),
    Data(&'a [u8]),
}

impl<'a> Src<'a> {
    fn of(tile: Option<&'a Tile>) -> Src<'a> {
        match tile {
            None => Src::Absent,
            Some(Tile::Uniform(c)) => Src::Uniform(c.to_array()),
            Some(Tile::Data(d)) => Src::Data(&d[..]),
        }
    }
}

/// 1 つのタイルの、層ごとの有無と読み元（ワーカーごとに 1 つ）。
struct TileState<'a> {
    present: Vec<bool>,
    pixels: Vec<Src<'a>>,
    masks: Vec<Src<'a>>,
}

/// 計算する矩形（タイルの中の位置: 左の画素 x、下の行 y、行数、画素数）。
#[derive(Clone, Copy)]
struct Geom {
    x: usize,
    y: usize,
    rows: usize,
    count: usize,
}

/// 深さごとの作業の矩形: 通過のグループの下の写し・グループやクリッピングの下地・クリッピングされたグループ。
#[derive(Default)]
struct Level {
    inner: Vec<u8>,
    group: Vec<u8>,
    clip: Vec<u8>,
}

const ZERO4: [u8; 4] = [0; 4];

impl<'a> Plan<'a> {
    /// タイルを読み込む（C# の Node.Load）。nodes の下に画素（ラスター・塗りつぶし）があれば true。
    fn load(&self, ids: &[usize], coord: TileCoord, st: &mut TileState<'a>) -> bool {
        let mut any = false;
        for &id in ids {
            let n = &self.nodes[id];
            let mut pixels;
            match &n.content {
                Content::Group => {
                    pixels = self.load(&n.children, coord, st);
                    st.present[id] = pixels
                        || n.children.iter().any(|&c| {
                            st.present[c]
                                && matches!(
                                    self.nodes[c].content,
                                    Content::Adjust(_) | Content::Group
                                )
                        });
                }
                Content::Adjust(_) => {
                    st.present[id] = true;
                    pixels = false;
                }
                Content::Raster(s) => {
                    let src = Src::of(s.tile(coord));
                    st.present[id] = !matches!(src, Src::Absent);
                    st.pixels[id] = src;
                    pixels = st.present[id];
                }
                Content::Fill(value) => {
                    st.present[id] = value.is_some();
                    st.pixels[id] = value.map_or(Src::Absent, |c| Src::Uniform(c.to_array()));
                    pixels = st.present[id];
                }
            }
            if st.present[id] {
                if let Some((m, _)) = &n.mask {
                    st.masks[id] = match Src::of(m.tile(coord)) {
                        Src::Absent => Src::Uniform(ZERO4), // 無いタイル = 何も隠さない
                        s => s,
                    };
                }
                pixels |= self.load(&n.clips, coord, st);
            }
            any |= pixels;
        }
        any
    }

    fn rows<'s>(&self, src: &'s Src<'a>, g: Geom) -> Rows<'s> {
        match src {
            Src::Data(d) => Rows {
                bytes: d,
                off: (g.y * self.tile_size + g.x) * 4,
                stride: self.tile_size * 4,
                step: 4,
            },
            Src::Uniform(c) => Rows {
                bytes: c,
                off: 0,
                stride: 0,
                step: 0,
            },
            Src::Absent => unreachable!("無いタイルは読まない"),
        }
    }

    fn amount<'s>(&'s self, id: usize, g: Geom, st: &'s TileState<'a>) -> Amount<'s> {
        let n = &self.nodes[id];
        Amount {
            opacity: n.opacity,
            mask: n
                .mask
                .as_ref()
                .map(|(_, f)| (self.rows(&st.masks[id], g), &**f)),
        }
    }

    /// 矩形の行を res（行の並びは out）へ重ねる（C# の EvaluateRect）。
    #[allow(clippy::too_many_arguments)]
    fn eval_rect(
        &self,
        ids: &[usize],
        res: &mut [u8],
        out: Out,
        g: Geom,
        st: &TileState<'a>,
        scratch: &mut [Level],
    ) {
        let packed = g.count * 4;
        let area = g.rows * packed;
        for &id in ids {
            if !st.present[id] {
                continue;
            }
            let n = &self.nodes[id];
            let amount = self.amount(id, g, st);
            if let Content::Adjust(k) = &n.content {
                adjust_rows(res, out, g, k, amount, n.raw_mode);
                continue;
            }
            let (level, deeper) = scratch.split_first_mut().expect("深さの分の作業の矩形");
            if n.passes_through {
                if n.mask.is_none() && n.opacity >= 1.0 {
                    // 不透明度 1 でマスクの効かない通過グループ: フェードは中身そのものなので、下の結果へそのまま重ねる
                    self.eval_rect(&n.children, res, out, g, st, deeper);
                    continue;
                }
                let inner = grow(&mut level.inner, area);
                for r in 0..g.rows {
                    inner[r * packed..(r + 1) * packed]
                        .copy_from_slice(&res[out.row(r)..out.row(r) + packed]);
                }
                self.eval_rect(&n.children, inner, Out::packed(packed), g, st, deeper);
                fade_rows(res, out, inner, g, amount, self.normal);
                continue;
            }
            // 下地: グループは中身を透明から、クリッピングの組は層の画素の写し、ほかは層の画素をそのまま読む
            let base: &mut [u8] = match &n.content {
                Content::Group => {
                    let gb = grow(&mut level.group, area);
                    gb.fill(0);
                    self.eval_rect(&n.children, gb, Out::packed(packed), g, st, deeper);
                    gb
                }
                _ if !n.clips.is_empty() => {
                    let gb = grow(&mut level.group, area);
                    let src = self.rows(&st.pixels[id], g);
                    for r in 0..g.rows {
                        let row = src.row(r);
                        let out = &mut gb[r * packed..(r + 1) * packed];
                        if src.step == 0 {
                            for p in out.chunks_exact_mut(4) {
                                p.copy_from_slice(&row[..4]);
                            }
                        } else {
                            out.copy_from_slice(&row[..packed]);
                        }
                    }
                    gb
                }
                _ => {
                    let src = self.rows(&st.pixels[id], g);
                    blend_rows(res, out, src, g, amount, n.mode, n.table, self.normal);
                    continue;
                }
            };
            for &cid in &n.clips {
                if !st.present[cid] {
                    continue;
                }
                let c = &self.nodes[cid];
                let camount = self.amount(cid, g, st);
                if let Content::Adjust(k) = &c.content {
                    adjust_rows(base, Out::packed(packed), g, k, camount, c.raw_mode);
                    continue;
                }
                let over = if let Content::Group = c.content {
                    let cb = grow(&mut level.clip, area);
                    cb.fill(0);
                    self.eval_rect(&c.children, cb, Out::packed(packed), g, st, deeper);
                    Rows {
                        bytes: &level.clip[..area],
                        off: 0,
                        stride: packed,
                        step: 4,
                    }
                } else {
                    self.rows(&st.pixels[cid], g)
                };
                for r in 0..g.rows {
                    let row = &mut base[r * packed..(r + 1) * packed];
                    if self.normal {
                        normal_clip_span(row, over.row(r), over.step, camount.row(r), c.mode);
                    } else {
                        clip_span(row, over.row(r), over.step, camount.row(r), c.mode, c.table);
                    }
                }
            }
            let over = Rows {
                bytes: base,
                off: 0,
                stride: packed,
                step: 4,
            };
            blend_rows(res, out, over, g, amount, n.mode, n.table, self.normal);
        }
    }
}

/// 作業の矩形（使うときに広げる）。
#[inline]
fn grow(v: &mut Vec<u8>, n: usize) -> &mut [u8] {
    if v.len() < n {
        v.resize(n, 0);
    }
    &mut v[..n]
}

#[allow(clippy::too_many_arguments)]
#[inline]
fn blend_rows(
    res: &mut [u8],
    out: Out,
    over: Rows<'_>,
    g: Geom,
    amount: Amount<'_>,
    mode: BlendMode,
    table: Option<&[f64]>,
    normal: bool,
) {
    let packed = g.count * 4;
    for r in 0..g.rows {
        let row = &mut res[out.row(r)..out.row(r) + packed];
        if normal {
            normal_blend_span(row, over.row(r), over.step, amount.row(r), mode);
        } else {
            blend_span(row, over.row(r), over.step, amount.row(r), mode, table);
        }
    }
}

fn adjust_rows(
    res: &mut [u8],
    out: Out,
    g: Geom,
    k: &AdjustKernel,
    amount: Amount<'_>,
    mode: BlendMode,
) {
    for r in 0..g.rows {
        let a = amount.row(r);
        let row = &mut res[out.row(r)..out.row(r) + g.count * 4];
        for (i, p) in row.chunks_exact_mut(4).enumerate() {
            let v = k.composite(Rgba8::from_slice(p), a.at(i), mode);
            p.copy_from_slice(&v.to_array());
        }
    }
}

fn fade_rows(res: &mut [u8], out: Out, inner: &[u8], g: Geom, amount: Amount<'_>, normal: bool) {
    let packed = g.count * 4;
    for r in 0..g.rows {
        let a = amount.row(r);
        let row = &mut res[out.row(r)..out.row(r) + packed];
        let inn = &inner[r * packed..(r + 1) * packed];
        for (i, p) in row.chunks_exact_mut(4).enumerate() {
            let v = stack_fade(
                normal,
                Rgba8::from_slice(p),
                Rgba8::from_slice(&inn[i * 4..]),
                a.at(i),
            );
            p.copy_from_slice(&v.to_array());
        }
    }
}

/// これより仕事（画素 × 層）が少ない合成は、呼んだスレッドだけで行う（ワーカーを起こす方が高くつく。C# と同じ目安）。
const PARALLEL_MINIMUM_WORK: u64 = 1 << 16;

/// ワーカーごとの道具（タイルの状態と深さごとの作業の矩形）。
struct Worker<'a> {
    st: TileState<'a>,
    scratch: Vec<Level>,
}

impl<'a> Worker<'a> {
    fn new(nodes: usize, depth: usize) -> Self {
        Worker {
            st: TileState {
                present: vec![false; nodes],
                pixels: vec![Src::Absent; nodes],
                masks: vec![Src::Absent; nodes],
            },
            // 作業の矩形は使うときに広げる（グループもクリッピングも無ければ確保しない）
            scratch: (0..depth + 1).map(|_| Level::default()).collect(),
        }
    }
}

/// 矩形の合成を out（width × height × 4、行の並びは order）へ書く。out の元の中身は使わない。
pub(crate) fn composite_into(
    stack: &Stack<'_>,
    tile_size: u32,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
) {
    composite_entries_into(stack, stack.plan(), tile_size, rect, out, order)
}

/// `composite_into` の、計画（段の並び）を渡す形。グループの中身だけを透明から重ねるとき（グループの出力）に、そのグループの子の計画を渡す。
pub(crate) fn composite_entries_into(
    stack: &Stack<'_>,
    entries: Vec<Entry>,
    tile_size: u32,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
) {
    if rect.is_empty() {
        return;
    }
    if entries.is_empty() {
        out.fill(0);
        return;
    }
    let mut plan = Plan::build(stack, &entries);
    plan.tile_size = tile_size as usize;
    let depth = plan_depth(&entries);
    let layer_count = count_entries(&entries).max(1);
    let work = rect.width as u64 * rect.height as u64 * layer_count;
    let threads = if work < PARALLEL_MINIMUM_WORK {
        1
    } else {
        rayon::current_num_threads().max(1)
    };

    // 行の束: タイルの行をまたがない帯を、スレッドが余らない数に割る
    let ts = tile_size;
    let (ry0, ry1) = (rect.y, rect.y + rect.height);
    let tile_rows = (ry1 - 1) / ts - ry0 / ts + 1;
    let pieces_per_tile_row = if threads <= 1 {
        1
    } else {
        ((threads as u32 * 2).div_ceil(tile_rows)).clamp(1, ts.div_ceil(8))
    };
    let mut bands: Vec<(u32, u32)> = Vec::new();
    for ty in ry0 / ts..=(ry1 - 1) / ts {
        let (y0, y1) = ((ty * ts).max(ry0), ((ty + 1) * ts).min(ry1));
        let h = y1 - y0;
        let step = h.div_ceil(pieces_per_tile_row).max(1);
        let mut y = y0;
        while y < y1 {
            let e = (y + step).min(y1);
            bands.push((y, e));
            y = e;
        }
    }
    // out を帯ごとの連続した行へ分ける（TopDown では上の帯が先）
    let row_bytes = rect.width as usize * 4;
    if order == RowOrder::TopDown {
        bands.reverse();
    }
    let mut chunks: Vec<((u32, u32), &mut [u8])> = Vec::with_capacity(bands.len());
    let mut rest = out;
    for &(y0, y1) in &bands {
        let (head, tail) = rest.split_at_mut((y1 - y0) as usize * row_bytes);
        chunks.push(((y0, y1), head));
        rest = tail;
    }
    let make = || Worker::new(plan.nodes.len(), depth);
    let plan = &plan;
    if threads <= 1 || chunks.len() <= 1 {
        let mut w = make();
        for ((y0, y1), chunk) in chunks {
            composite_band(plan, &mut w, rect, y0, y1, chunk, order);
        }
    } else {
        chunks
            .into_par_iter()
            .for_each_init(make, |w, ((y0, y1), chunk)| {
                composite_band(plan, w, rect, y0, y1, chunk, order)
            });
    }
}

/// 1 つの帯（タイルの行の中の y0..y1）を合成する。chunk は帯の行（order の並び）。
fn composite_band<'a>(
    plan: &Plan<'a>,
    w: &mut Worker<'a>,
    rect: Rect,
    y0: u32,
    y1: u32,
    chunk: &mut [u8],
    order: RowOrder,
) {
    chunk.fill(0); // 透明から（帯ごとに、受け持つスレッドが 0 で埋める）
    let ts = plan.tile_size as u32;
    let ty = y0 / ts;
    let rows = (y1 - y0) as usize;
    let row_bytes = rect.width as usize * 4;
    let (rx0, rx1) = (rect.x, rect.x + rect.width);
    for tx in rx0 / ts..=(rx1 - 1) / ts {
        let coord = TileCoord::new(tx, ty);
        let (x0, x1) = ((tx * ts).max(rx0), ((tx + 1) * ts).min(rx1));
        if !plan.load(&plan.roots, coord, &mut w.st) {
            continue; // 透明のまま
        }
        let g = Geom {
            x: (x0 - tx * ts) as usize,
            y: (y0 - ty * ts) as usize,
            rows,
            count: (x1 - x0) as usize,
        };
        let res = &mut chunk[(x0 - rx0) as usize * 4..];
        let out = Out {
            stride: row_bytes,
            flip: (order == RowOrder::TopDown).then_some(rows - 1),
        };
        plan.eval_rect(&plan.roots, res, out, g, &w.st, &mut w.scratch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::to_byte;

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }
        fn byte(&mut self) -> u8 {
            let r = self.next();
            match r & 7 {
                0 => 0,
                1 => 255,
                _ => (r >> 8) as u8,
            }
        }
    }

    fn whole(opacity: f64) -> RowAmount<'static> {
        RowAmount {
            opacity,
            mask: None,
        }
    }

    /// 行の核が画素ごとの式（blend・clip_onto）と同じバイトを出すか、近道の境（透明・不透明・量 1・極小の量）とマスクを含めて。
    #[test]
    fn spans_match_the_pixel_formulas() {
        let mut rng = Rng(7);
        let amounts = [
            1.0,
            0.7,
            0.5,
            1.0 / 255.0,
            1e-305,
            f64::from_bits(1),
            0.99999999,
        ];
        let mut factor = [0.0; 256];
        for (h, f) in factor.iter_mut().enumerate() {
            *f = 1.0 - 0.8 * UNIT[h];
        }
        for mode in BlendMode::LAYER_MODES {
            let table = separable_table(mode);
            for &amount in &amounts {
                let n = 512;
                let below: Vec<u8> = (0..n * 4).map(|_| rng.byte()).collect();
                let over: Vec<u8> = (0..n * 4).map(|_| rng.byte()).collect();
                let mask: Vec<u8> = (0..n * 4).map(|_| rng.byte()).collect();
                for masked in [false, true] {
                    let a = RowAmount {
                        opacity: amount,
                        mask: masked.then_some((&mask[..], 4, &factor)),
                    };
                    let mut res = below.clone();
                    blend_span(&mut res, &over, 4, a, mode, table);
                    let mut g = below.clone();
                    clip_span(&mut g, &over, 4, a, mode, table);
                    let mut nres = below.clone();
                    normal_blend_span(&mut nres, &over, 4, a, mode);
                    for i in 0..n {
                        let d = Rgba8::from_slice(&below[i * 4..]);
                        let s = Rgba8::from_slice(&over[i * 4..]);
                        let am = a.at(i);
                        assert_eq!(
                            Rgba8::from_slice(&res[i * 4..]),
                            blend(d, s, am, mode),
                            "{mode:?} {amount} {d:?} {s:?}"
                        );
                        assert_eq!(
                            Rgba8::from_slice(&g[i * 4..]),
                            clip_onto(d, s, am, mode),
                            "clip {mode:?} {amount}"
                        );
                        assert_eq!(
                            Rgba8::from_slice(&nres[i * 4..]),
                            normal::blend_unchecked(d, s, am, mode),
                            "normal {mode:?} {amount}"
                        );
                    }
                }
            }
        }
        // 一様な読み元（刻み 0）
        let mut res = vec![10u8, 20, 30, 128, 0, 0, 0, 0];
        blend_span(
            &mut res,
            &[200, 100, 50, 77],
            0,
            whole(1.0),
            BlendMode::Screen,
            None,
        );
        assert_eq!(
            Rgba8::from_slice(&res[4..]),
            Rgba8::new(200, 100, 50, to_byte(77.0 / 255.0))
        );
    }
}
