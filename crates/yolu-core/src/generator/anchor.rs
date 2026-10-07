//! Anchor は「そこまで」のスナップショット。入力の段は既にフィルター評価済みでなければならない。
use super::{evaluate::allocate, unit, Error, Kind, Options, Source};
use crate::{blend, math::UNIT, AdjustmentSettings, BlendMode, Channel, ChannelKind, Rect, Rgba8};
use rayon::prelude::*;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Issue {
    NotChosen,
    Missing,
    NotBelow,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    Layer,
    Mask,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Point {
    pub id: u128,
    pub host: usize,
    pub placement: Placement,
}
/// 下から上のレイヤー番号で参照を解決する。すべての辺が小さい番号を向くので循環は成立しない。
pub fn resolve(
    points: &[Point],
    id: u128,
    reader: usize,
    layer_count: usize,
) -> Result<Point, Issue> {
    if id == 0 {
        return Err(Issue::NotChosen);
    }
    let p = points.iter().find(|p| p.id == id).ok_or(Issue::Missing)?;
    if reader >= layer_count || p.host >= layer_count {
        return Err(Issue::Missing);
    }
    if p.host >= reader {
        return Err(Issue::NotBelow);
    }
    Ok(*p)
}
pub fn validate_points(points: &[Point], layer_count: usize) -> Result<(), Error> {
    for (i, p) in points.iter().enumerate() {
        if p.id == 0
            || p.host >= layer_count
            || points[..i]
                .iter()
                .any(|q| p.id == q.id || (p.host == q.host && p.placement == q.placement))
        {
            return Err(Error::Invalid(
                "Anchor の ID・レイヤー・配置が不正または重複しています",
            ));
        }
    }
    Ok(())
}
/// 値は有限な 0..1。評価中は同じ座標に同じ値を返す。
pub trait ValueSource: Sync {
    fn dimensions(&self) -> (u32, u32);
    fn value(&self, x: u32, y: u32) -> Option<f64>;
    /// 行 `y` の `x0` から `out.len()` 画素の値。値を持たない画素は `f64::NAN`（値は有限な 0..1 なので取り違えない）。
    /// 既定は `value` の繰り返し。連続した行をまとめて読める実装は置き換えられる。
    fn value_row(&self, x0: u32, y: u32, out: &mut [f64]) {
        for (i, o) in out.iter_mut().enumerate() {
            *o = self.value(x0 + i as u32, y).unwrap_or(f64::NAN);
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Read {
    Color,
    Scalar,
    Coverage,
}
/// 保存される読み方（C# の `AnchorRead`。値は追記のみ）。マスクの Anchor では無視する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ReadMode {
    /// 値×被覆（色のチャンネルは輝度×被覆）。
    Value = 0,
    /// 被覆（α）。
    Coverage = 1,
}
/// `Settings` が持つ Anchor の参照（C# の `GeneratorSettings` の AnchorId・AnchorChannel・AnchorRead。正本の版 20）。
/// `id` は [`Point::id`] で、0 は未選択。Anchor 以外の種類は既定値（0・Color・Value）のままにする。
/// レイヤーの Anchor は `channel` のスタックを `read` で読み、マスクの Anchor は `channel` と `read` を無視する。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reference {
    pub id: u128,
    pub channel: Channel,
    pub read: ReadMode,
}
impl Reference {
    /// 新しい設定の既定。Anchor は下のレイヤーの Height を読むのがいちばん多いので Height、他の種類は C# と同じ Color。
    pub(super) fn default_for(kind: Kind) -> Self {
        Self {
            id: 0,
            channel: if kind == Kind::Anchor {
                Channel::Height
            } else {
                Channel::Color
            },
            read: ReadMode::Value,
        }
    }
    /// 参照先のレイヤー番号を [`resolve`](self::resolve) で解決する。
    pub fn resolve(
        &self,
        points: &[Point],
        reader: usize,
        layer_count: usize,
    ) -> Result<Point, Issue> {
        resolve(points, self.id, reader, layer_count)
    }
    /// レイヤーの Anchor を読むときの [`Read`]。`kind` は `channel` の種類で、値の読み方は色なら輝度・スカラーなら R になる。
    /// Normal は 1 画素 1 値でないので断る。
    pub fn read_for(&self, kind: ChannelKind) -> Result<Read, Error> {
        match (self.read, kind) {
            (_, ChannelKind::Normal) => {
                Err(Error::Invalid("Normal チャンネルは Anchor で読めません"))
            }
            (ReadMode::Coverage, _) => Ok(Read::Coverage),
            (ReadMode::Value, ChannelKind::Color) => Ok(Read::Color),
            (ReadMode::Value, ChannelKind::Scalar) => Ok(Read::Scalar),
        }
    }
}
pub struct LayerSample<'a> {
    pub source: &'a dyn Source,
    pub read: Read,
}
impl LayerSample<'_> {
    fn read_value(&self, p: Rgba8) -> f64 {
        let a = UNIT[p.a as usize];
        match self.read {
            Read::Coverage => a,
            Read::Scalar => UNIT[p.r as usize] * a,
            Read::Color => {
                (0.3 * UNIT[p.r as usize] + 0.59 * UNIT[p.g as usize] + 0.11 * UNIT[p.b as usize])
                    * a
            }
        }
    }
}
/// 行をまとめて読むときの 1 回の画素数。
const ROW_CHUNK: usize = 256;
impl ValueSource for LayerSample<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.source.dimensions()
    }
    fn value(&self, x: u32, y: u32) -> Option<f64> {
        let (w, h) = self.dimensions();
        if x >= w || y >= h {
            return None;
        }
        Some(self.read_value(self.source.pixel(x, y)))
    }
    fn value_row(&self, x0: u32, y: u32, out: &mut [f64]) {
        let (w, h) = self.dimensions();
        let inside = if y >= h || x0 >= w {
            0
        } else {
            out.len().min((w - x0) as usize)
        };
        let (head, tail) = out.split_at_mut(inside);
        tail.fill(f64::NAN);
        let mut bytes = [0u8; ROW_CHUNK * 4];
        for (n, chunk) in head.chunks_mut(ROW_CHUNK).enumerate() {
            let bytes = &mut bytes[..chunk.len() * 4];
            self.source.read_row(x0 + (n * ROW_CHUNK) as u32, y, bytes);
            for (o, p) in chunk.iter_mut().zip(bytes.chunks_exact(4)) {
                *o = self.read_value(Rgba8::from_slice(p));
            }
        }
    }
}
#[derive(Clone, Copy)]
pub struct Mask<'a> {
    pub source: &'a dyn Source,
    pub enabled: bool,
    pub inverted: bool,
    pub density: f64,
}
impl Mask<'_> {
    fn validate(&self, d: (u32, u32)) -> Result<(), Error> {
        if !unit(self.density) || self.source.dimensions() != d {
            Err(Error::Invalid("マスクの濃度・大きさが不正です"))
        } else {
            Ok(())
        }
    }
    pub fn factor(&self, x: u32, y: u32) -> f64 {
        if !self.enabled {
            return 1.;
        }
        self.factor_of_alpha(self.source.pixel(x, y).a)
    }
    /// マスクの画素のアルファ `a` から見える量（有効なマスク用。無効なら 1）。
    fn factor_of_alpha(&self, a: u8) -> f64 {
        if !self.enabled {
            return 1.;
        }
        let h = UNIT[a as usize];
        if self.inverted {
            1. - self.density * (1. - h)
        } else {
            1. - self.density * h
        }
    }
}
pub struct MaskSample<'a> {
    mask: Mask<'a>,
}
impl<'a> MaskSample<'a> {
    pub fn new(mask: Mask<'a>) -> Result<Self, Error> {
        mask.validate(mask.source.dimensions())?;
        Ok(Self { mask })
    }
}
impl ValueSource for MaskSample<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.mask.source.dimensions()
    }
    fn value(&self, x: u32, y: u32) -> Option<f64> {
        let (w, h) = self.dimensions();
        (x < w && y < h).then(|| self.mask.factor(x, y))
    }
    fn value_row(&self, x0: u32, y: u32, out: &mut [f64]) {
        let (w, h) = self.dimensions();
        let inside = if y >= h || x0 >= w {
            0
        } else {
            out.len().min((w - x0) as usize)
        };
        let (head, tail) = out.split_at_mut(inside);
        tail.fill(f64::NAN);
        if !self.mask.enabled {
            head.fill(1.);
            return;
        }
        let mut bytes = [0u8; ROW_CHUNK * 4];
        for (n, chunk) in head.chunks_mut(ROW_CHUNK).enumerate() {
            let bytes = &mut bytes[..chunk.len() * 4];
            self.mask
                .source
                .read_row(x0 + (n * ROW_CHUNK) as u32, y, bytes);
            for (o, p) in chunk.iter_mut().zip(bytes.chunks_exact(4)) {
                *o = self.mask.factor_of_alpha(p[3]);
            }
        }
    }
}
/// 1 チャンネルの評価済み入力。グループのフィルターは統合側で画像へ評価して渡す。
pub enum Content<'a> {
    Pixels(&'a dyn Source),
    Fill(Rgba8),
    Group,
    Adjustment(AdjustmentSettings),
}
pub struct Layer<'a> {
    pub parent: Option<usize>,
    pub content: Content<'a>,
    pub visible: bool,
    pub enabled: bool,
    pub opacity: f64,
    pub blend: BlendMode,
    pub clipping: bool,
    pub mask: Option<Mask<'a>>,
}
impl<'a> Layer<'a> {
    pub fn new(content: Content<'a>) -> Self {
        Self {
            parent: None,
            content,
            visible: true,
            enabled: true,
            opacity: 1.,
            blend: BlendMode::Normal,
            clipping: false,
            mask: None,
        }
    }
}
#[derive(Clone)]
struct Entry {
    layer: usize,
    children: Vec<Entry>,
    clips: Vec<Entry>,
}
/// 祖先の不透明度・表示・マスクを適用せず、分離グループの境界で透明に戻す合成計画。
pub struct Plan<'a> {
    layers: &'a [Layer<'a>],
    entries: Vec<Entry>,
    dimensions: (u32, u32),
    /// 評価するチャンネルの種類（トーンカーブの調整が色とスカラーで変わる）。
    kind: ChannelKind,
}
impl<'a> Plan<'a> {
    pub fn new(
        layers: &'a [Layer<'a>],
        host: usize,
        dimensions: (u32, u32),
        kind: ChannelKind,
    ) -> Result<Self, Error> {
        if host >= layers.len()
            || dimensions.0 == 0
            || dimensions.1 == 0
            || kind == ChannelKind::Normal
        {
            return Err(Error::Invalid(
                "Anchor のレイヤー・大きさ・チャンネルが不正です",
            ));
        }
        for (i, l) in layers.iter().enumerate() {
            if !unit(l.opacity)
                || l.parent.is_some_and(|p| {
                    p >= layers.len() || p == i || !matches!(layers[p].content, Content::Group)
                })
                || (!matches!(l.content, Content::Group) && l.blend == BlendMode::PassThrough)
            {
                return Err(Error::Invalid("Anchor のレイヤーの設定が不正です"));
            }
            if let Content::Pixels(s) = l.content {
                if s.dimensions() != dimensions {
                    return Err(Error::Invalid("Anchor の画像サイズが違います"));
                }
            }
            if let Some(m) = l.mask {
                m.validate(dimensions)?;
            }
            let mut cur = l.parent;
            let mut depth = 0;
            while let Some(p) = cur {
                depth += 1;
                if depth > 256 || p == i || p >= layers.len() {
                    return Err(Error::Invalid("グループの循環または深さ超過です"));
                }
                cur = layers[p].parent;
            }
        }
        let mut p = Self {
            layers,
            entries: vec![],
            dimensions,
            kind,
        };
        let mut path = vec![host];
        let mut current = layers[host].parent;
        while let Some(i) = current {
            path.push(i);
            current = layers[i].parent;
        }
        path.reverse();
        for (depth, i) in path.iter().enumerate() {
            let last = depth == path.len() - 1;
            let (level, clipped) = p.level(layers[*i].parent, Some((*i, last)), kind);
            if !last && (clipped || layers[*i].blend != BlendMode::PassThrough) {
                p.entries.clear();
            } else {
                p.entries.extend(level);
            }
        }
        Ok(p)
    }
    fn entry(&self, i: usize, kind: ChannelKind) -> Option<Entry> {
        let l = &self.layers[i];
        if !l.visible || l.opacity <= 0. {
            return None;
        }
        let children = if matches!(l.content, Content::Group) {
            let children = self.level(Some(i), None, kind).0;
            if children.is_empty() {
                return None;
            }
            children
        } else {
            if !l.enabled {
                return None;
            }
            if let Content::Adjustment(a) = &l.content {
                if !a.applies_to(kind) {
                    return None;
                }
            }
            vec![]
        };
        Some(Entry {
            layer: i,
            children,
            clips: vec![],
        })
    }
    fn level(
        &self,
        parent: Option<usize>,
        stop: Option<(usize, bool)>,
        kind: ChannelKind,
    ) -> (Vec<Entry>, bool) {
        let mut entries: Vec<Entry> = vec![];
        let mut base: Option<usize> = None;
        let mut takes = false;
        let mut clipped = false;
        for (ordinal, (i, l)) in self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.parent == parent)
            .enumerate()
        {
            let at = stop.is_some_and(|(s, _)| s == i);
            if at {
                clipped = ordinal > 0 && l.clipping;
                if !stop.unwrap().1 {
                    break;
                }
            }
            if ordinal > 0 && l.clipping {
                if takes {
                    if let Some(b) = base {
                        if let Some(e) = self.entry(i, kind) {
                            entries[b].clips.push(e);
                        }
                    }
                }
            } else {
                takes = !matches!(l.content, Content::Adjustment(_));
                base = self.entry(i, kind).map(|e| {
                    entries.push(e);
                    entries.len() - 1
                });
            }
            if at {
                break;
            }
        }
        (entries, clipped)
    }
    fn amount(&self, i: usize, x: u32, y: u32) -> f64 {
        let l = &self.layers[i];
        l.opacity * l.mask.map_or(1., |m| m.factor(x, y))
    }
    fn raw(&self, e: &Entry, x: u32, y: u32) -> Rgba8 {
        match self.layers[e.layer].content {
            Content::Pixels(s) => s.pixel(x, y),
            Content::Fill(p) => p,
            Content::Group => self.composite(&e.children, Rgba8::TRANSPARENT, x, y),
            Content::Adjustment(_) => unreachable!(),
        }
    }
    fn composite(&self, entries: &[Entry], mut below: Rgba8, x: u32, y: u32) -> Rgba8 {
        for e in entries {
            let l = &self.layers[e.layer];
            let amount = self.amount(e.layer, x, y);
            if let Content::Adjustment(a) = &l.content {
                below = a.composite_in(self.kind, below, amount, l.blend);
                continue;
            }
            if matches!(l.content, Content::Group)
                && l.blend == BlendMode::PassThrough
                && e.clips.is_empty()
            {
                below = blend::fade(below, self.composite(&e.children, below, x, y), amount);
                continue;
            }
            let mut group = self.raw(e, x, y);
            for clip in &e.clips {
                let l = &self.layers[clip.layer];
                let a = self.amount(clip.layer, x, y);
                group = if let Content::Adjustment(adj) = &l.content {
                    adj.composite_in(self.kind, group, a, l.blend)
                } else {
                    blend::clip_onto(group, self.raw(clip, x, y), a, l.blend)
                };
            }
            below = blend::blend(below, group, amount, l.blend);
        }
        below
    }
    pub fn evaluate(&self, region: Rect, options: &Options<'_>) -> Result<Vec<u8>, Error> {
        let mut pixels = allocate(region, self.dimensions, options)?;
        pixels
            .par_chunks_mut(region.width as usize * 4)
            .enumerate()
            .try_for_each(|(row, bytes)| -> Result<(), Error> {
                options.check()?;
                for (col, dst) in bytes.chunks_exact_mut(4).enumerate() {
                    dst.copy_from_slice(
                        &self
                            .pixel(region.x + col as u32, region.y + row as u32)
                            .to_array(),
                    );
                }
                Ok(())
            })?;
        options.check()?;
        Ok(pixels)
    }
}
impl Source for Plan<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.dimensions
    }
    fn pixel(&self, x: u32, y: u32) -> Rgba8 {
        self.composite(&self.entries, Rgba8::TRANSPARENT, x, y)
    }
}
