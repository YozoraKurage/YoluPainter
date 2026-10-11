//! CPU の合成（C# の CpuCompositor の Plan・EvaluatePixel と、タイルの経路の Node.Load・EvaluateRect・BlendRect・ClipRect）。
//!
//! - レイヤーは下から上へ、透明から始めて 1 段ずつ重ね、段ごとに RGBA8 へ丸める。
//! - 計画（[`plan`]）: 兄弟の中で一番下でなくクリッピングの印のあるレイヤーは、すぐ下の印の無い兄弟（下地）の組に入る。見えない下地
//!   （非表示・そのチャンネルの不透明度 0・中身が無い）は組ごと落とす。調整レイヤーは下地にならない（その上のクリッピングは描かない）。
//!   グループは中身の計画を持ち、見えないグループ・中身の無いグループは落とす。
//! - 通過のグループ（PassThrough でクリッピングの組を持たない）は中身を下の結果へ重ね、不透明度 × マスクで下とフェードする
//!   （不透明度 1 でマスクが効かなければ、フェードは中身そのものなので下へそのまま重ねる）。ほかのグループは中身を透明から
//!   合成して、レイヤーと同じように重ねる（クリッピングされたグループは通過の指定でも透明から）。
//! - マスクの量はレイヤーのアルファに掛ける（不透明度 × マスクの値）。何も変えないマスク（無効・濃度 0・空）は掛けない（値はちょうど 1）。
//! - タイルの経路は、そのタイルに何も無いレイヤー（グループは中身に画素も調整も無いもの）を飛ばす（C# と同じ）。
//! - Normal の種類のチャンネルは [`crate::normal`] のベクトルの式で、ほかは色の式で重ねる。調整はどちらも色の式。
//! - 画素の値は自分の入力だけで決まるので、どのスレッドがどの行を受け持っても同じバイトになる。
//! - タイルの経路は行ごとの核（[`crate::blend::blend_row`] など。実行時に AVX2・SSE4.1・スカラー（aarch64 は NEON・スカラー）を選ぶ）で重ねる。画素ごとの参照
//!   （`evaluate_pixel`）と同じバイトで、歩幅つきの読み（粗い合成）は 64 画素ずつ詰めて核へ渡す。

use std::sync::{Arc, OnceLock};

use rayon::prelude::*;

mod memo;
pub use memo::MemoStats;
pub(crate) use memo::{Memo, MemoRequest};

use crate::adjust::AdjustKernel;
use crate::blend::{blend, blend_row, clip_onto, clip_row, fade, fade_row, RowAmount};
use crate::document::EvalSet;
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::normal;
use crate::surface::{Readers, Surface, Tile};
use crate::types::{BlendMode, Channel, ChannelKind, LayerKind, Rect, Rgba8, RowOrder, TileCoord};

/// 合成の 1 段: レイヤー（またはグループ）と、その組に入るクリッピングされたレイヤー（下から上）。グループは中身の計画を持つ。
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

/// レイヤーそのものの画素（`Layer::pixel` の、面を readers で読む形。ディスクから読めない画素は透明で、readers が誤りを覚える）。
pub(crate) fn raw_layer_pixel<'a>(
    readers: &mut Readers<'a>,
    layer: &'a Layer,
    channel: Channel,
    x: u32,
    y: u32,
) -> Rgba8 {
    match layer.kind {
        LayerKind::Raster => layer
            .surface(channel)
            .map_or(Rgba8::TRANSPARENT, |s| readers.pixel(s, x, y)),
        LayerKind::Fill => layer.fill_value(channel).unwrap_or(Rgba8::TRANSPARENT),
        _ => Rgba8::TRANSPARENT,
    }
}

/// レイヤーの並びと、親ごとの子の並び（下から上）。
pub(crate) struct Stack<'a> {
    pub layers: &'a [Layer],
    children: std::collections::HashMap<Option<LayerId>, Vec<usize>>,
    channel: Channel,
    kind: ChannelKind,
    /// 評価したレイヤーの出力（フィルター・Generator・画像・グラデーションを通したもの）。あれば、元の画素の代わりに読む。
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

    /// 評価した面が粗く評価したもの（`EvalSet::reduced`）なら、その歩幅。
    pub(crate) fn reduced_by(&self) -> Option<usize> {
        self.eval.and_then(|e| e.reduced).map(|s| s as usize)
    }

    /// レイヤー（番号）の評価済みの出力の面（評価が要らないレイヤーは None）。
    fn evaluated_content(&self, layer: usize) -> Option<&'a Surface> {
        self.eval.and_then(|e| e.content.get(&layer))
    }
    /// レイヤーのマスクの評価済みの隠す量の面（マスクにフィルターが無ければ None）。
    fn evaluated_mask(&self, layer: usize) -> Option<&'a Surface> {
        self.eval.and_then(|e| e.masks.get(&layer))
    }
    /// レイヤーの画素（マスク・不透明度・合成の前。評価した出力があればそれ。中身の無い所・調整・グループは透明）。面は readers で読む
    /// （ディスクから読めない画素は透明で、readers が誤りを覚える）。
    pub(crate) fn layer_pixel(
        &self,
        readers: &mut Readers<'a>,
        layer: usize,
        x: u32,
        y: u32,
    ) -> Rgba8 {
        match self.evaluated_content(layer) {
            Some(s) => readers.pixel(s, x, y),
            None => raw_layer_pixel(readers, &self.layers[layer], self.channel, x, y),
        }
    }
    /// レイヤーのマスクがレイヤーのアルファに掛ける値。
    pub(crate) fn mask_factor(
        &self,
        readers: &mut Readers<'a>,
        layer: usize,
        x: u32,
        y: u32,
    ) -> f64 {
        match &self.layers[layer].mask {
            None => 1.0,
            Some(m) => {
                let surface = self.evaluated_mask(layer).unwrap_or(&m.surface);
                m.factor(readers.pixel(surface, x, y).a)
            }
        }
    }

    fn siblings(&self, parent: Option<LayerId>) -> &[usize] {
        self.children.get(&parent).map_or(&[], |v| v.as_slice())
    }

    /// そのレイヤーがこのチャンネルで何かを出せるか（C# の Active）。
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

pub(crate) fn evaluate_pixel<'a>(
    stack: &Stack<'a>,
    readers: &mut Readers<'a>,
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
        let amount = entry.opacity * stack.mask_factor(readers, entry.layer, x, y);
        if layer.kind == LayerKind::Adjustment {
            let a = layer.adjustment.as_ref().expect("調整レイヤーは設定を持つ");
            result = a.composite_in(stack.kind, result, amount, entry.mode);
            continue;
        }
        if entry.passes_through(layers) {
            let inner = evaluate_pixel(stack, readers, &entry.children, result, x, y);
            result = stack_fade(normal, result, inner, amount);
            continue;
        }
        let mut group = if layer.is_group() {
            evaluate_pixel(stack, readers, &entry.children, Rgba8::TRANSPARENT, x, y)
        } else {
            stack.layer_pixel(readers, entry.layer, x, y)
        };
        for clip in &entry.clips {
            let c = &layers[clip.layer];
            let clip_amount = clip.opacity * stack.mask_factor(readers, clip.layer, x, y);
            if c.kind == LayerKind::Adjustment {
                let a = c.adjustment.as_ref().expect("調整レイヤーは設定を持つ");
                group = a.composite_in(stack.kind, group, clip_amount, clip.mode);
            } else {
                let over = if c.is_group() {
                    evaluate_pixel(stack, readers, &clip.children, Rgba8::TRANSPARENT, x, y)
                } else {
                    stack.layer_pixel(readers, clip.layer, x, y)
                };
                group = stack_clip(normal, group, over, clip_amount, clip.blend_mode());
            }
        }
        result = stack_blend(normal, result, group, amount, entry.blend_mode());
    }
    result
}

/// 画素 1 つの合成（C# の CompositePixel。タイルの経路と照らし合わせる参照の式）。
pub(crate) fn composite_pixel(stack: &Stack<'_>, x: u32, y: u32) -> Result<Rgba8, CoreError> {
    let mut readers = Readers::default();
    let p = evaluate_pixel(stack, &mut readers, &stack.plan(), Rgba8::TRANSPARENT, x, y);
    readers.finish()?;
    Ok(p)
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

impl<'a> Amount<'a> {
    #[inline]
    fn row(&self, r: usize) -> RowAmount<'a> {
        RowAmount {
            opacity: self.opacity,
            mask: self.mask.map(|(m, f)| (m.row(r), m.step, f)),
        }
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

/// 画素の面のタイルの読み方: タイルの一辺と、拾う歩幅（1 なら全画素。粗い合成は歩幅ごとに 1 画素）。
#[derive(Clone, Copy, Debug)]
struct Geo {
    tile: usize,
    sample: usize,
}

struct Node<'a> {
    content: Content<'a>,
    /// 画素の面の読み方。
    px: Geo,
    /// マスクの面の読み方。
    mask_px: Geo,
    opacity: f64,
    /// 重ねるモード（PassThrough は Normal）。
    mode: BlendMode,
    /// 計画のモードそのもの（調整の合成に使う）。
    raw_mode: BlendMode,
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
    /// 粗い合成の歩幅（1 なら全画素）。
    stride: usize,
    /// タイルをディスクから読めなかったときの誤り（読めなかったタイルは無いものとして進め、終わりに誤りを返す）。
    failed: OnceLock<CoreError>,
}

impl Node<'_> {
    /// 1 画素あたりの仕事の重み: 不透明で 1 つ上書きするだけのレイヤーは 1、重ね方の計算が要るレイヤー・調整・グループは 6（実測で、128² のタイル 1 枚が
    /// 通常モードの不透明なレイヤー 24 枚で 0.4 ms 前後、モードや不透明度の違うレイヤー 24 枚で 2.7 ms 前後）。
    fn weight(&self) -> u64 {
        let copy = matches!(self.content, Content::Raster(_) | Content::Fill(_))
            && self.mode == BlendMode::Normal
            && self.mask.is_none()
            && self.opacity >= 1.0
            && self.clips.is_empty();
        if copy {
            1
        } else {
            6
        }
    }
}

impl<'a> Plan<'a> {
    fn build(stack: &Stack<'a>, plan: &[Entry], tile_size: u32) -> Plan<'a> {
        Self::build_strided(stack, plan, 1, tile_size as usize)
    }

    /// 歩幅 stride で拾う計画。`tile_size` は文書のタイルの一辺（評価した面が粗く評価したもの（`EvalSet::reduced`）なら、その面のタイルは
    /// 一辺が tile_size / stride で、全画素を読む）。
    fn build_strided(
        stack: &Stack<'a>,
        plan: &[Entry],
        stride: usize,
        tile_size: usize,
    ) -> Plan<'a> {
        let mut p = Plan {
            nodes: Vec::new(),
            roots: Vec::new(),
            normal: stack.kind == ChannelKind::Normal,
            tile_size,
            stride,
            failed: OnceLock::new(),
        };
        p.roots = plan.iter().map(|e| p.add(stack, e)).collect();
        p
    }

    fn add(&mut self, stack: &Stack<'a>, e: &Entry) -> usize {
        let layers: &'a [Layer] = stack.layers;
        let layer: &'a Layer = &layers[e.layer];
        // 評価した出力の面（粗く評価したものは、タイルの一辺が歩幅の分だけ小さく、全画素を読む）か、レイヤーの保存した画素の面
        let reduced = stack.reduced_by();
        let geo_of = |evaluated: bool, this: &Self| -> Geo {
            match reduced {
                Some(s) if evaluated => Geo {
                    tile: this.tile_size / s,
                    sample: 1,
                },
                _ => Geo {
                    tile: this.tile_size,
                    sample: this.stride,
                },
            }
        };
        let mut px = geo_of(false, self);
        let content = match layer.kind {
            LayerKind::Raster => match stack.evaluated_content(e.layer) {
                Some(surface) => {
                    px = geo_of(true, self);
                    Content::Raster(surface)
                }
                None => Content::Raster(
                    layer
                        .surface(stack.channel)
                        .expect("計画のレイヤーは面を持つ"),
                ),
            },
            LayerKind::Fill => match stack.evaluated_content(e.layer) {
                Some(surface) => {
                    px = geo_of(true, self);
                    Content::Raster(surface)
                }
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
                    .expect("調整レイヤーは設定を持つ")
                    .kernel(stack.kind),
            ),
            LayerKind::Group => Content::Group,
        };
        let mode = e.blend_mode();
        let mut mask_px = geo_of(false, self);
        let mask = layer.mask.as_ref().filter(|m| !m.is_neutral()).map(|m| {
            let evaluated = stack.evaluated_mask(e.layer);
            if evaluated.is_some() {
                mask_px = geo_of(true, self);
            }
            (evaluated.unwrap_or(&m.surface), Box::new(m.factor_table()))
        });
        let id = self.nodes.len();
        self.nodes.push(Node {
            content,
            px,
            mask_px,
            opacity: e.opacity,
            mode,
            raw_mode: e.mode,
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

/// あるタイルでのレイヤーの画素・マスクの読み元（全画素のタイルは読んだ中身を持つ。持っている間はディスクへ逃がさない）。
#[derive(Clone)]
enum Src {
    Absent,
    Uniform([u8; 4]),
    Data(Arc<Vec<u8>>),
}

/// 1 つのタイルの、レイヤーごとの有無と読み元（ワーカーごとに 1 つ）。
struct TileState {
    present: Vec<bool>,
    pixels: Vec<Src>,
    masks: Vec<Src>,
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
    /// タイルの読み元。ディスクから読めなければ誤りを覚えて無いものとする（合成の終わりに `result` が誤りを返す）。
    fn src(&self, tile: Option<&Tile>) -> Src {
        match tile {
            None => Src::Absent,
            Some(Tile::Uniform(c)) => Src::Uniform(c.to_array()),
            Some(Tile::Data(d)) => match d.bytes() {
                Ok(bytes) => Src::Data(bytes),
                Err(e) => {
                    let _ = self.failed.set(e);
                    Src::Absent
                }
            },
        }
    }

    /// 合成の間のタイルが全部読めたか。
    fn result(&self) -> Result<(), CoreError> {
        self.failed.get().map_or(Ok(()), |e| Err(e.clone()))
    }

    /// タイルを読み込む（C# の Node.Load）。nodes の下に画素（ラスター・塗りつぶし）があれば true。
    fn load(&self, ids: &[usize], coord: TileCoord, st: &mut TileState) -> bool {
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
                    let src = self.src(s.tile(coord));
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
                    st.masks[id] = match self.src(m.tile(coord)) {
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

    /// 矩形の仕事の見積り（画素 × そのタイルで画素のあるレイヤー・調整の重みの和）。タイルが少ない矩形は、タイルごとに読んで本当に画素のあるレイヤーを数える
    /// （空のタイルの多い文書で、小さな合成を分けて高くつくのを避ける）。多い矩形はどのタイルも詰まっているとみなす（`layer_count` 枚）。
    fn work(&self, rect: Rect, layer_count: u64, run: Option<&memo::MemoRun>) -> u64 {
        let ts = self.tile_size as u32;
        let (tx0, tx1) = (rect.x / ts, (rect.x + rect.width - 1) / ts);
        let (ty0, ty1) = (rect.y / ts, (rect.y + rect.height - 1) / ts);
        let tiles = (tx1 - tx0 + 1) as u64 * (ty1 - ty0 + 1) as u64;
        if tiles > WORK_SCAN_TILES {
            return rect.width as u64 * rect.height as u64 * layer_count;
        }
        let n = self.nodes.len();
        let mut st = TileState {
            present: vec![false; n],
            pixels: vec![Src::Absent; n],
            masks: vec![Src::Absent; n],
        };
        let (rx1, ry1) = (rect.x + rect.width, rect.y + rect.height);
        let mut work = 0u64;
        for ty in ty0..=ty1 {
            for tx in tx0..=tx1 {
                st.present.fill(false);
                if !self.load(&self.roots, TileCoord::new(tx, ty), &mut st) {
                    continue; // 透明のまま（合成しない）
                }
                let w = ((tx + 1) * ts).min(rx1) - (tx * ts).max(rect.x);
                let h = ((ty + 1) * ts).min(ry1) - (ty * ts).max(rect.y);
                // 覚えから続けるタイルは、描くレイヤー以降の段だけが仕事
                let only = run.and_then(|r| r.suffix_of(TileCoord::new(tx, ty)));
                let weight: u64 = st
                    .present
                    .iter()
                    .enumerate()
                    .filter(|(i, p)| **p && only.is_none_or(|m| m[*i]))
                    .map(|(i, _)| self.nodes[i].weight())
                    .sum();
                work += w as u64 * h as u64 * weight;
            }
        }
        work
    }

    fn rows<'s>(&self, src: &'s Src, g: Geom, geo: Geo) -> Rows<'s> {
        match src {
            Src::Data(d) => Rows {
                bytes: &d[..],
                off: (g.y * geo.sample * geo.tile + g.x * geo.sample) * 4,
                stride: geo.tile * geo.sample * 4,
                step: 4 * geo.sample,
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

    fn amount<'s>(&'s self, id: usize, g: Geom, st: &'s TileState) -> Amount<'s> {
        let n = &self.nodes[id];
        Amount {
            opacity: n.opacity,
            mask: n
                .mask
                .as_ref()
                .map(|(_, f)| (self.rows(&st.masks[id], g, n.mask_px), &**f)),
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
        st: &TileState,
        scratch: &mut [Level],
    ) {
        for &id in ids {
            self.eval_node(id, res, out, g, st, scratch, None);
        }
    }

    /// 段の子の並びを重ねる。描いているレイヤーへの道をたどるとき（`resume` がある）は、その段の手前までの覚えから続ける。
    #[allow(clippy::too_many_arguments)]
    fn eval_children(
        &self,
        ids: &[usize],
        res: &mut [u8],
        out: Out,
        g: Geom,
        st: &TileState,
        scratch: &mut [Level],
        resume: Option<memo::Resume<'_>>,
    ) {
        match resume {
            None => self.eval_rect(ids, res, out, g, st, scratch),
            Some(r) => self.eval_resume(ids, r, res, out, g, st, scratch),
        }
    }

    /// 段 1 つを res へ重ねる。`resume` は、この段が描いているレイヤーへの道の途中の段（グループ）のとき、その中身の評価を覚えから続ける印。
    #[allow(clippy::too_many_arguments)]
    fn eval_node(
        &self,
        id: usize,
        res: &mut [u8],
        out: Out,
        g: Geom,
        st: &TileState,
        scratch: &mut [Level],
        resume: Option<memo::Resume<'_>>,
    ) {
        if !st.present[id] {
            return;
        }
        let packed = g.count * 4;
        let area = g.rows * packed;
        let n = &self.nodes[id];
        let amount = self.amount(id, g, st);
        if let Content::Adjust(k) = &n.content {
            adjust_rows(res, out, g, k, amount, n.raw_mode);
            return;
        }
        let (level, deeper) = scratch.split_first_mut().expect("深さの分の作業の矩形");
        if n.passes_through {
            if n.mask.is_none() && n.opacity >= 1.0 {
                // 不透明度 1 でマスクの効かない通過グループ: フェードは中身そのものなので、下の結果へそのまま重ねる
                self.eval_children(&n.children, res, out, g, st, deeper, resume);
                return;
            }
            let inner = grow(&mut level.inner, area);
            for r in 0..g.rows {
                inner[r * packed..(r + 1) * packed]
                    .copy_from_slice(&res[out.row(r)..out.row(r) + packed]);
            }
            self.eval_children(
                &n.children,
                inner,
                Out::packed(packed),
                g,
                st,
                deeper,
                resume,
            );
            fade_rows(res, out, inner, g, amount, self.normal);
            return;
        }
        // 下地: グループは中身を透明から、クリッピングの組はレイヤーの画素の写し、ほかはレイヤーの画素をそのまま読む
        let base: &mut [u8] = match &n.content {
            Content::Group => {
                let gb = grow(&mut level.group, area);
                gb.fill(0);
                self.eval_children(&n.children, gb, Out::packed(packed), g, st, deeper, resume);
                gb
            }
            _ if !n.clips.is_empty() => {
                let gb = grow(&mut level.group, area);
                let src = self.rows(&st.pixels[id], g, n.px);
                for r in 0..g.rows {
                    let row = src.row(r);
                    let out = &mut gb[r * packed..(r + 1) * packed];
                    if src.step == 0 {
                        for p in out.chunks_exact_mut(4) {
                            p.copy_from_slice(&row[..4]);
                        }
                    } else if src.step == 4 {
                        out.copy_from_slice(&row[..packed]);
                    } else {
                        // 歩幅つきの読み（粗い合成）: 画素を間引いて写す
                        for (i, p) in out.chunks_exact_mut(4).enumerate() {
                            p.copy_from_slice(&row[i * src.step..i * src.step + 4]);
                        }
                    }
                }
                gb
            }
            _ => {
                let src = self.rows(&st.pixels[id], g, n.px);
                blend_rows(res, out, src, g, amount, n.mode, self.normal);
                return;
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
                self.rows(&st.pixels[cid], g, c.px)
            };
            let mode = c.mode;
            for r in 0..g.rows {
                let row = &mut base[r * packed..(r + 1) * packed];
                if self.normal {
                    over_row(
                        row,
                        over.row(r),
                        over.step,
                        camount.row(r),
                        |g, s, step, a| normal::clip_row(g, s, step, a, mode),
                    );
                } else {
                    over_row(
                        row,
                        over.row(r),
                        over.step,
                        camount.row(r),
                        |g, s, step, a| clip_row(g, s, step, a, mode),
                    );
                }
            }
        }
        let over = Rows {
            bytes: base,
            off: 0,
            stride: packed,
            step: 4,
        };
        blend_rows(res, out, over, g, amount, n.mode, self.normal);
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

/// 矩形の行ごとに、読み元（over）を res の行へ重ねる（`normal` なら Normal チャンネルの式）。
#[inline]
fn blend_rows(
    res: &mut [u8],
    out: Out,
    over: Rows<'_>,
    g: Geom,
    amount: Amount<'_>,
    mode: BlendMode,
    normal: bool,
) {
    let packed = g.count * 4;
    for r in 0..g.rows {
        let row = &mut res[out.row(r)..out.row(r) + packed];
        if normal {
            over_row(
                row,
                over.row(r),
                over.step,
                amount.row(r),
                |d, s, step, a| normal::blend_row(d, s, step, a, mode),
            );
        } else {
            over_row(
                row,
                over.row(r),
                over.step,
                amount.row(r),
                |d, s, step, a| blend_row(d, s, step, a, mode),
            );
        }
    }
}

/// 歩幅つきの読み（粗い合成）を詰めて渡す 1 回の画素数。
const GATHER: usize = 64;

/// 行の核（読み元の刻みが 0 か 4 のときだけ SIMD の道を通る）へ 1 行を渡す。刻みが 4 より大きい読み元（粗い合成）は、`GATHER` 画素ずつ
/// 詰めて刻み 4 で渡し、量のマスクの読みも同じだけ進める（詰める費用は画素 1 つの 4 バイトの複写）。
#[inline]
fn over_row(
    dst: &mut [u8],
    src: &[u8],
    step: usize,
    amount: RowAmount<'_>,
    kernel: impl Fn(&mut [u8], &[u8], usize, RowAmount<'_>),
) {
    if step == 0 || step == 4 {
        return kernel(dst, src, step, amount);
    }
    let mut packed = [0u8; GATHER * 4];
    for (k, part) in dst.chunks_mut(GATHER * 4).enumerate() {
        let (first, n) = (k * GATHER, part.len() / 4);
        for (i, p) in packed.chunks_exact_mut(4).take(n).enumerate() {
            let at = (first + i) * step;
            p.copy_from_slice(&src[at..at + 4]);
        }
        kernel(part, &packed[..n * 4], 4, amount.offset(first));
    }
}

/// 矩形の行ごとに、調整の核を res の行へ当てる。
fn adjust_rows(
    res: &mut [u8],
    out: Out,
    g: Geom,
    k: &AdjustKernel,
    amount: Amount<'_>,
    mode: BlendMode,
) {
    for r in 0..g.rows {
        let row = &mut res[out.row(r)..out.row(r) + g.count * 4];
        k.composite_row(row, amount.row(r), mode);
    }
}

/// 矩形の行ごとに、中身（inner）と res の行を量でフェードさせる（通過のグループ）。
fn fade_rows(res: &mut [u8], out: Out, inner: &[u8], g: Geom, amount: Amount<'_>, normal: bool) {
    let packed = g.count * 4;
    for r in 0..g.rows {
        let a = amount.row(r);
        let row = &mut res[out.row(r)..out.row(r) + packed];
        let inn = &inner[r * packed..(r + 1) * packed];
        if normal {
            normal::fade_row(row, inn, a);
        } else {
            fade_row(row, inn, a);
        }
    }
}

/// これより仕事（画素 × 画素のあるレイヤー）が少ない合成は、呼んだスレッドだけで行う（ワーカーを起こす・分けたタイルを読み直す費用が、
/// 計算より高くつく。1 タイル 128² を 7 レイヤーで 0.1 ms 足らずの計算に、8 本へ分ける費用が 0.4 ms 前後かかる）。
const PARALLEL_MINIMUM_WORK: u64 = 1 << 19;

/// 1 つの帯に持たせる仕事の下限（これより小さく割らない）。
const MINIMUM_BAND_WORK: u64 = 1 << 18;

/// 仕事を数えるためにタイルを読んで見る上限の枚数（これより多い矩形は、どのタイルも詰まっているとみなす）。
const WORK_SCAN_TILES: u64 = 256;

/// ワーカーごとのツール（タイルの状態と深さごとの作業の矩形）。
struct Worker {
    st: TileState,
    scratch: Vec<Level>,
}

impl Worker {
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
) -> Result<(), CoreError> {
    composite_entries_into(stack, &stack.plan(), tile_size, rect, out, order)
}

/// `composite_into` の、描いている間の下の覚えを使える形（覚えが使えない・描くレイヤーが計画に無いときは `composite_into` と同じ道）。
pub(crate) fn composite_into_memo(
    stack: &Stack<'_>,
    tile_size: u32,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
    memo: &MemoRequest<'_>,
) -> Result<(), CoreError> {
    composite_entries_with(
        stack,
        &stack.plan(),
        tile_size,
        rect,
        out,
        order,
        Some(memo),
    )
}

/// `composite_into` の、計画（段の並び）を渡す形。グループの中身だけを透明から重ねるとき（グループの出力）に、そのグループの子の計画を渡す。
pub(crate) fn composite_entries_into(
    stack: &Stack<'_>,
    entries: &[Entry],
    tile_size: u32,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
) -> Result<(), CoreError> {
    composite_entries_with(stack, entries, tile_size, rect, out, order, None)
}

fn composite_entries_with(
    stack: &Stack<'_>,
    entries: &[Entry],
    tile_size: u32,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
    memo: Option<&MemoRequest<'_>>,
) -> Result<(), CoreError> {
    if rect.is_empty() {
        return Ok(());
    }
    if entries.is_empty() {
        out.fill(0);
        return Ok(());
    }
    let plan = Plan::build(stack, entries, tile_size);
    let depth = plan_depth(entries);
    let layer_count = count_entries(entries).max(1);
    let run = memo.and_then(|m| {
        let ts = tile_size;
        let coords: Vec<TileCoord> = (rect.y / ts..=(rect.y + rect.height - 1) / ts)
            .flat_map(|ty| {
                (rect.x / ts..=(rect.x + rect.width - 1) / ts).map(move |tx| TileCoord::new(tx, ty))
            })
            .collect();
        memo::prepare(&plan, entries, m, &coords, depth)
    });
    let work = plan.work(rect, layer_count, run.as_ref());
    composite_plan_into(&plan, depth, work, rect, out, order, run.as_ref());
    plan.result()
}

/// 計画を、矩形を行の帯に割って重ねる（仕事が小さければ、呼んだスレッドだけで）。work は `Plan::work` の見積り。
#[allow(clippy::too_many_arguments)]
fn composite_plan_into(
    plan: &Plan<'_>,
    depth: usize,
    work: u64,
    rect: Rect,
    out: &mut [u8],
    order: RowOrder,
    run: Option<&memo::MemoRun>,
) {
    let tile_size = plan.tile_size as u32;
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
        let by_threads = (threads as u32 * 2).div_ceil(tile_rows);
        // 小さな帯に割りすぎると、帯ごとのタイルの読み直しと分ける費用が計算を上回る
        let by_work = (work / MINIMUM_BAND_WORK / tile_rows as u64).max(1) as u32;
        by_threads.min(by_work).clamp(1, ts.div_ceil(8))
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
    if threads <= 1 || chunks.len() <= 1 {
        let mut w = make();
        for ((y0, y1), chunk) in chunks {
            composite_band(plan, &mut w, rect, y0, y1, chunk, order, run);
        }
    } else {
        chunks
            .into_par_iter()
            .for_each_init(make, |w, ((y0, y1), chunk)| {
                composite_band(plan, w, rect, y0, y1, chunk, order, run)
            });
    }
}

/// 散らばったタイルの合成（1 つの計画を使い回し、タイルごとにワーカーへ分ける）。regions のどれも 1 枚のタイルの中の矩形で、合成した
/// straight RGBA8（行は下から上）を、矩形ごとの仕上げ `finish(矩形の番号, 合成した画素)` へ渡し、結果は regions と同じ並びの仕上げの戻り値。
/// 矩形ごとに合成の関数を呼ぶ（その度に計画を作り、タイルの行ごとに帯へ分ける）より、計画を 1 回で組み、タイルをまとめてワーカーへ分けるので、
/// 1 枚あたりの時間が短い（比は文書と負荷による。計測の台 `comp2d_bench --only call` の「1 枚ずつ ÷ 束」）。画素の値は `composite_into` と
/// 同じ（矩形の分け方によらない）。仕上げはその矩形を合成したワーカーがそのまま行う。合成のあとに別の並列の段を挟むと、ワーカーを起こして
/// 待つ費用が段の数だけかかるので、画素を別の形（圧縮したタイルなど）へ変える・比べる仕事は仕上げに入れる。
pub(crate) fn composite_tiles_with<R: Send>(
    stack: &Stack<'_>,
    tile_size: u32,
    regions: &[Rect],
    memo: Option<&MemoRequest<'_>>,
    finish: impl Fn(usize, Vec<u8>) -> R + Sync,
) -> Result<Vec<R>, CoreError> {
    let size = |r: &Rect| r.width as usize * r.height as usize * 4;
    let entries = stack.plan();
    if entries.is_empty() {
        return Ok(regions
            .iter()
            .enumerate()
            .map(|(i, r)| finish(i, vec![0u8; size(r)]))
            .collect());
    }
    let plan = Plan::build(stack, &entries, tile_size);
    let depth = plan_depth(&entries);
    let nodes = plan.nodes.len();
    let run = memo.and_then(|m| {
        let coords: Vec<TileCoord> = regions
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| TileCoord::new(r.x / tile_size, r.y / tile_size))
            .collect();
        memo::prepare(&plan, &entries, m, &coords, depth)
    });
    let run = run.as_ref();
    fn one<'a>(plan: &Plan<'a>, w: &mut Worker, r: &Rect, run: Option<&memo::MemoRun>) -> Vec<u8> {
        let mut out = vec![0u8; r.width as usize * r.height as usize * 4];
        if !r.is_empty() {
            let ts = plan.tile_size as u32;
            debug_assert!(r.x / ts == (r.x + r.width - 1) / ts);
            debug_assert!(r.y / ts == (r.y + r.height - 1) / ts);
            composite_band(
                plan,
                w,
                *r,
                r.y,
                r.y + r.height,
                &mut out,
                RowOrder::BottomUp,
                run,
            );
        }
        out
    }
    let plan = &plan;
    let results = if regions.len() < PARALLEL_MINIMUM_TILES {
        // 少ない枚数でも、1 枚の仕事が大きければ（詰まった文書）、そのタイルを帯に割ってワーカーへ分ける
        let layer_count = count_entries(&entries).max(1);
        let mut w = Worker::new(nodes, depth);
        regions
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let work = if r.is_empty() {
                    0
                } else {
                    plan.work(*r, layer_count, run)
                };
                finish(
                    i,
                    if work >= PARALLEL_MINIMUM_WORK {
                        let mut out = vec![0u8; r.width as usize * r.height as usize * 4];
                        composite_plan_into(
                            plan,
                            depth,
                            work,
                            *r,
                            &mut out,
                            RowOrder::BottomUp,
                            run,
                        );
                        out
                    } else {
                        one(plan, &mut w, r, run)
                    },
                )
            })
            .collect()
    } else {
        regions
            .par_iter()
            .enumerate()
            .map_init(
                || Worker::new(nodes, depth),
                |w, (i, r)| finish(i, one(plan, w, r, run)),
            )
            .collect()
    };
    plan.result()?;
    Ok(results)
}

/// 散らばったタイルの、歩幅 stride で拾った粗い合成（表示の仮の絵）。regions のどれも 1 枚のタイルの中の矩形で、結果は regions と同じ並びの
/// straight RGBA8（行は下から上、幅・高さは矩形を stride で切り上げて割った値）。画素 (i, j) は矩形の左下から (i·stride, j·stride) の画素の
/// 合成と同じバイト（効果の出力は、評価した面の種類による。`Document::composite_coarse_tiles`）。stride は 1 以上でタイルの一辺の約数。
pub(crate) fn composite_coarse_tiles_into(
    stack: &Stack<'_>,
    tile_size: u32,
    regions: &[Rect],
    stride: u32,
) -> Result<Vec<Vec<u8>>, CoreError> {
    let dims = |r: &Rect| {
        (
            r.width.div_ceil(stride) as usize,
            r.height.div_ceil(stride) as usize,
        )
    };
    let entries = stack.plan();
    if entries.is_empty() {
        return Ok(regions
            .iter()
            .map(|r| {
                let (w, h) = dims(r);
                vec![0u8; w * h * 4]
            })
            .collect());
    }
    let plan = Plan::build_strided(stack, &entries, stride as usize, tile_size as usize);
    let depth = plan_depth(&entries);
    let nodes = plan.nodes.len();
    fn one<'a>(plan: &Plan<'a>, w: &mut Worker, r: &Rect, stride: u32) -> Vec<u8> {
        let (ow, oh) = (
            r.width.div_ceil(stride) as usize,
            r.height.div_ceil(stride) as usize,
        );
        let mut out = vec![0u8; ow * oh * 4];
        if r.is_empty() {
            return out;
        }
        let ts = plan.tile_size as u32;
        debug_assert!(r.x / ts == (r.x + r.width - 1) / ts);
        debug_assert!(r.y / ts == (r.y + r.height - 1) / ts);
        if !plan.load(&plan.roots, TileCoord::new(r.x / ts, r.y / ts), &mut w.st) {
            return out;
        }
        let g = Geom {
            x: 0,
            y: 0,
            rows: oh,
            count: ow,
        };
        plan.eval_rect(
            &plan.roots,
            &mut out,
            Out::packed(ow * 4),
            g,
            &w.st,
            &mut w.scratch,
        );
        out
    }
    let plan = &plan;
    let results = if regions.len() < PARALLEL_MINIMUM_TILES {
        let mut w = Worker::new(nodes, depth);
        regions
            .iter()
            .map(|r| one(plan, &mut w, r, stride))
            .collect()
    } else {
        regions
            .par_iter()
            .map_init(|| Worker::new(nodes, depth), |w, r| one(plan, w, r, stride))
            .collect()
    };
    plan.result()?;
    Ok(results)
}

/// タイルの束がこの枚数より少ないときは、呼んだスレッドだけで合成する（1 枚が 0.1 ms 前後で、ワーカーを起こす費用に見合わない）。
const PARALLEL_MINIMUM_TILES: usize = 4;

/// 1 つの帯（タイルの行の中の y0..y1）を合成する。chunk は帯の行（order の並び）。
#[allow(clippy::too_many_arguments)]
fn composite_band<'a>(
    plan: &Plan<'a>,
    w: &mut Worker,
    rect: Rect,
    y0: u32,
    y1: u32,
    chunk: &mut [u8],
    order: RowOrder,
    memo: Option<&memo::MemoRun>,
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
        match memo.and_then(|m| m.resume(coord)) {
            // 描くレイヤーより下は覚えから、描くレイヤーと上のレイヤーだけを重ねる
            Some(r) => plan.eval_resume(&plan.roots, r, res, out, g, &w.st, &mut w.scratch),
            None => plan.eval_rect(&plan.roots, res, out, g, &w.st, &mut w.scratch),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::math::to_byte;
    use crate::math::UNIT;

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

    /// 粗い合成の確かめ用の文書: 3 × 1 枚のタイル（右端は部分。3 枚以下のタイルは呼んだスレッドだけで合成する）に、合成モード・不透明度・
    /// マスク・クリッピング・通過のグループ・調整・塗りつぶし・Normal のチャンネルを重ねる。
    fn coarse_document() -> Document {
        use crate::adjust::AdjustmentSettings;
        let mut doc = Document::with_tile_size(300, 100, 128).unwrap();
        let mut rng = Rng(11);
        let ts = doc.tile_size() as usize;
        let coords: Vec<TileCoord> = doc.canvas_tiles().collect();
        let mut random_tiles = |doc: &mut Document, id: LayerId, channel: Channel, mask: bool| {
            let mut bytes = vec![0u8; ts * ts * 4];
            for c in &coords {
                for (i, p) in bytes.chunks_exact_mut(4).enumerate() {
                    // キャンバスの外の余白は 0（読み込みの決まり）
                    let x = c.x * ts as u32 + (i % ts) as u32;
                    let y = c.y * ts as u32 + (i / ts) as u32;
                    if x >= doc.width() || y >= doc.height() {
                        p.copy_from_slice(&[0; 4]);
                        continue;
                    }
                    let a = rng.byte();
                    let rgb = if mask {
                        [0, 0, 0]
                    } else {
                        [rng.byte(), rng.byte(), rng.byte()]
                    };
                    p.copy_from_slice(&[rgb[0], rgb[1], rgb[2], a]);
                }
                if mask {
                    doc.import_mask_tile(id, *c, &bytes).unwrap();
                } else {
                    doc.import_tile(id, channel, *c, &bytes).unwrap();
                }
            }
        };
        let modes = [
            BlendMode::Normal,
            BlendMode::Multiply,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::ColorDodge,
            BlendMode::Hue,
            BlendMode::Luminosity,
            BlendMode::Screen,
        ];
        let mut ids = Vec::new();
        for (i, mode) in modes.into_iter().enumerate() {
            let id = doc.add_layer(&format!("l{i}")).unwrap();
            random_tiles(&mut doc, id, Channel::Color, false);
            doc.set_layer_blend_mode(id, mode).unwrap();
            doc.set_layer_opacity(id, [1.0, 0.7, 0.35][i % 3], false)
                .unwrap();
            if i % 3 == 1 {
                doc.add_layer_mask(id).unwrap();
                random_tiles(&mut doc, id, Channel::Color, true);
            }
            if i % 4 == 2 {
                doc.set_layer_clipping(id, true).unwrap();
            }
            if i % 2 == 0 {
                doc.set_channel_enabled(id, Channel::Normal, true).unwrap();
                random_tiles(&mut doc, id, Channel::Normal, false);
            }
            ids.push(id);
        }
        doc.add_adjustment_layer(
            "levels",
            AdjustmentSettings::levels(0.1, 0.9, 1.4, 0.05, 0.95).unwrap(),
            None,
            None,
        )
        .unwrap();
        let group = doc.group_layers(&ids[5..8], "group").unwrap();
        doc.set_layer_opacity(group, 0.6, false).unwrap();
        let fill = doc
            .add_fill_layer(
                "fill",
                &[(Channel::Color, Rgba8::new(30, 90, 200, 255))],
                None,
            )
            .unwrap();
        doc.set_layer_blend_mode(fill, BlendMode::Multiply).unwrap();
        doc.set_layer_opacity(fill, 0.25, false).unwrap();
        doc
    }

    /// 粗い合成（読み元の刻みが 4 より大きい）は、歩幅つきの読み元を詰めてから行の核へ渡す。歩幅つきのまま渡すとスカラーの道に落ちて遅くなるので、
    /// 核が SIMD に入れない刻みで呼ばれないことと、画素が全体の合成のその位置の画素と同じバイトであることを、歩幅・チャンネル・タイルの端で確かめる。
    #[test]
    fn a_coarse_composite_hands_the_row_kernels_only_packed_sources() {
        // 対照: 歩幅つきの読み元を核へ直に渡すと数える（数える仕組みが働いていることの確かめ）
        let (mut dst, src) = (vec![0u8; 16 * 4], vec![255u8; 16 * 8]);
        let before = crate::blend::scalar_steps::count();
        blend_row(&mut dst, &src, 8, whole(1.0), BlendMode::Normal);
        assert_eq!(crate::blend::scalar_steps::count(), before + 1);

        let doc = coarse_document();
        let coords: Vec<TileCoord> = doc.canvas_tiles().collect();
        assert!(
            coords.len() < PARALLEL_MINIMUM_TILES,
            "呼んだスレッドだけで合成する"
        );
        let before = crate::blend::scalar_steps::count();
        for channel in [Channel::Color, Channel::Normal] {
            for stride in [2, 4, 8, 16] {
                for tile in doc
                    .composite_coarse_tiles(channel, &coords, stride)
                    .unwrap()
                {
                    let (w, h) = tile.size();
                    for j in 0..h {
                        for i in 0..w {
                            let (x, y) = (tile.rect.x + i * stride, tile.rect.y + j * stride);
                            let want = doc.composite_pixel(channel, x, y).unwrap().to_array();
                            let at = ((j * w + i) * 4) as usize;
                            assert_eq!(
                                &tile.pixels[at..at + 4],
                                &want,
                                "{channel:?} 歩幅 {stride} ({x},{y})"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(
            crate::blend::scalar_steps::count(),
            before,
            "粗い合成が、歩幅つきの読み元を行の核へ直に渡した"
        );
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
                    blend_row(&mut res, &over, 4, a, mode);
                    let mut g = below.clone();
                    clip_row(&mut g, &over, 4, a, mode);
                    let mut nres = below.clone();
                    normal::blend_row(&mut nres, &over, 4, a, mode);
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
        blend_row(
            &mut res,
            &[200, 100, 50, 77],
            0,
            whole(1.0),
            BlendMode::Screen,
        );
        assert_eq!(
            Rgba8::from_slice(&res[4..]),
            Rgba8::new(200, 100, 50, to_byte(77.0 / 255.0))
        );
    }
}
