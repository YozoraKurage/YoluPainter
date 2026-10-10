//! 効果（フィルターのスタック・Generator・Anchor・塗りつぶしの画像と投影・グラデーション）の編集と Undo
//! （C# の `PaintDocument.Filters / Generators / Anchors / FillImages / FillGradients` の編集の口）。
//!
//! どれも 1 回の Undo で、断ったら何も変えない。スライダーのドラッグ（強さ・設定・投影・グラデーション）はまとめられる。
//! 評価は [`super::eval`]（合成のとき）。ここは設定の入れ物の変更と、変化の記録（そのレイヤーが出す所を作り直させる）まで。

use std::collections::BTreeMap;

use super::{CoalesceKey, Command, Document};
use crate::effects::{
    require_standard, value_type_of, Anchor, AnchorId, AnchorInfo, AnchorIssue, AnchorIssueKind,
    AnchorPlacement, EffectInputs, EffectSettings, FallbackEffect, FilterEffect, FilterId,
    FilterSpec, FilterTarget, ImageId, InactiveEffect, InactiveReason, InactiveTarget,
    MAX_FILTERS_PER_STACK, MAX_FILTER_STACK_HALO,
};
use crate::error::CoreError;
use crate::fill_image::{ImageMipChain, Projection, ProjectionMode};
use crate::fill_points::{PointGradient, PointSpace};
use crate::filter::{self, ValueType};
use crate::generator::{self, anchor};
use crate::layer::{Layer, LayerId};
use crate::math::require_finite;
use crate::types::{Channel, ChannelKind, LayerKind, Rgba8};

/// 塗りつぶしのチャンネルの、画像・グラデーション・値・有効の組（1 回の Undo で一緒に戻す）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FillChannelState {
    pub value: Option<Rgba8>,
    pub enabled: bool,
    pub image: Option<ImageId>,
    /// 画像を異方性のフィルターで読むか（画像の無いチャンネルは既定の true）。
    pub anisotropic: bool,
    pub gradient: Option<generator::Settings>,
    pub points: Option<PointGradient>,
}

impl FillChannelState {
    pub(super) fn of(layer: &Layer, channel: Channel) -> Self {
        FillChannelState {
            value: layer.fill.get(&channel).copied(),
            enabled: layer.is_channel_enabled(channel),
            image: layer.fill_images.get(&channel).copied(),
            anisotropic: layer.fill_anisotropic(channel),
            gradient: layer.fill_gradients.get(&channel).cloned(),
            points: layer.fill_points.get(&channel).cloned(),
        }
    }
    fn put(&self, layer: &mut Layer, channel: Channel) {
        match self.value {
            Some(v) => layer.fill.insert(channel, v),
            None => layer.fill.remove(&channel),
        };
        match self.image {
            Some(i) => layer.fill_images.insert(channel, i),
            None => layer.fill_images.remove(&channel),
        };
        // 画像の読み方は画像と一緒にだけ持つ（画像の無いチャンネルは既定）
        if self.anisotropic || self.image.is_none() {
            layer.fill_isotropic.remove(&channel);
        } else {
            layer.fill_isotropic.insert(channel);
        }
        match &self.points {
            Some(g) => layer.fill_points.insert(channel, g.clone()),
            None => layer.fill_points.remove(&channel),
        };
        match &self.gradient {
            Some(g) => layer.fill_gradients.insert(channel, g.clone()),
            None => layer.fill_gradients.remove(&channel),
        };
        layer.set_enabled(channel, self.enabled);
    }
}

/// 画像・グラデーションを足したのに値が無いチャンネルが見せる値（C# の `DefaultFillImageFallback`）。
pub(crate) fn default_fill_fallback(kind: ChannelKind) -> Rgba8 {
    match kind {
        ChannelKind::Normal => Rgba8::new(128, 128, 255, 255),
        ChannelKind::Scalar => Rgba8::new(128, 128, 128, 255),
        ChannelKind::Color => Rgba8::new(255, 255, 255, 255),
    }
}

/// 段の設定が使うブロックの作業バイト数の見積もり用の段の並び。
pub(super) fn stages_of(chain: &[&FilterEffect]) -> Vec<filter::Stage> {
    chain
        .iter()
        .map(|e| {
            let settings = match &e.settings {
                EffectSettings::Filter(f) => f.clone(),
                EffectSettings::Generator(g) => filter::Settings::Generator {
                    slot: 0,
                    blend: generator_blend(g.blend),
                },
            };
            filter::Stage {
                settings,
                enabled: e.enabled,
                strength: e.strength,
            }
        })
        .collect()
}

/// generator の合成を、フィルターの評価器の合成へ（同じ並び）。
pub(crate) fn generator_blend(b: generator::Blend) -> filter::GeneratorBlend {
    match b {
        generator::Blend::Multiply => filter::GeneratorBlend::Multiply,
        generator::Blend::Replace => filter::GeneratorBlend::Replace,
        generator::Blend::Screen => filter::GeneratorBlend::Screen,
        generator::Blend::Max => filter::GeneratorBlend::Max,
        generator::Blend::Min => filter::GeneratorBlend::Min,
        generator::Blend::Add => filter::GeneratorBlend::Add,
        generator::Blend::Subtract => filter::GeneratorBlend::Subtract,
    }
}

impl Document {
    // ───────── 参照 ─────────

    /// ID の段と、それがあるレイヤー・スタック（内容・マスクの順に探す）。
    pub fn find_filter(&self, id: FilterId) -> Option<(LayerId, &FilterEffect, FilterTarget)> {
        for l in &self.layers {
            if let Some(e) = l.filters.iter().find(|e| e.id == id) {
                return Some((l.id, e, FilterTarget::Content));
            }
            if let Some(e) = l
                .mask
                .as_ref()
                .and_then(|m| m.filters.iter().find(|e| e.id == id))
            {
                return Some((l.id, e, FilterTarget::Mask));
            }
        }
        None
    }

    /// レイヤーの、そのスタック（内容・マスク）の段。マスクのスタックはマスクが無ければ空。
    pub fn filters_of(
        &self,
        layer: LayerId,
        target: FilterTarget,
    ) -> Result<&[FilterEffect], CoreError> {
        let l = self.layer(layer).ok_or(CoreError::LayerNotFound)?;
        Ok(match target {
            FilterTarget::Content => &l.filters,
            FilterTarget::Mask => l.mask.as_ref().map_or(&[], |m| &m.filters),
        })
    }

    fn stack_ref(
        &self,
        index: usize,
        target: FilterTarget,
    ) -> Result<&Vec<FilterEffect>, CoreError> {
        let l = &self.layers[index];
        match target {
            FilterTarget::Content => Ok(&l.filters),
            FilterTarget::Mask => l
                .mask
                .as_ref()
                .map(|m| &m.filters)
                .ok_or(CoreError::Unsupported("レイヤーにマスクが無い")),
        }
    }

    fn new_filter_id(&mut self) -> FilterId {
        loop {
            self.id_counter += 1;
            let id = FilterId(super::random_id(self.id_counter));
            if id.0 != 0 && self.find_filter(id).is_none() {
                return id;
            }
        }
    }
    fn new_anchor_id(&mut self) -> AnchorId {
        loop {
            self.id_counter += 1;
            let id = AnchorId(super::random_id(self.id_counter));
            if id.0 != 0 && self.find_anchor(id).is_none() {
                return id;
            }
        }
    }

    // ───────── 写し ─────────

    /// 結合の結果へ写した、残すマスクの段に新しい ID を付ける（C# の `CloneMask` は段に新しい ID を付ける。マスクの Anchor は同じものを
    /// 保つので触らない）。段の ID を持つレイヤーは文書に無いまま、`self`（準備用の文書）の数で決める。
    pub(super) fn renew_kept_mask_filter_ids(&mut self, result: &mut Layer) {
        if let Some(m) = &mut result.mask {
            for e in &mut m.filters {
                e.id = self.new_filter_id();
            }
        }
    }

    /// レイヤーの写し（まだ文書に入れていない）の段・Anchor に新しい ID を付ける。写しの中の Anchor を読む段は写しの Anchor を読む（外の Anchor
    /// への参照はそのまま）。レイヤーを写す操作（複製・グループの写し）が、写しを文書へ入れる前に呼ぶ。
    pub(super) fn renew_effect_ids(&mut self, copies: &mut [Layer]) {
        // 定規にも新しい ID（文書の中で重ならない）
        self.renew_ruler_ids(copies);
        let mut anchors: std::collections::HashMap<u128, u128> = std::collections::HashMap::new();
        for l in copies.iter_mut() {
            let mut stacks: Vec<&mut Vec<FilterEffect>> = vec![&mut l.filters];
            if let Some(m) = &mut l.mask {
                stacks.push(&mut m.filters);
            }
            for stack in stacks {
                for e in stack.iter_mut() {
                    e.id = self.new_filter_id();
                    // 次の ID が同じにならないよう、この段の ID を持つレイヤーは文書に無いので、数の進みで分ける（new_filter_id は進める）
                }
            }
            let mut renew = |a: &mut Option<Anchor>| {
                if let Some(a) = a {
                    let new = self.new_anchor_id();
                    anchors.insert(a.id.0, new.0);
                    a.id = new;
                }
            };
            renew(&mut l.anchor);
            if let Some(m) = &mut l.mask {
                renew(&mut m.anchor);
            }
            // パスにも新しい ID（C# の複製と同じ。一覧のパスごとに別の ID）
            for (k, e) in l.paths.iter_mut().enumerate() {
                let id = super::random_id(
                    self.id_counter
                        .wrapping_add(0x5041_5448)
                        .wrapping_add(k as u64),
                );
                match &mut e.path {
                    crate::effects::LayerPath::Canvas(p) => p.id = id,
                    crate::effects::LayerPath::Surface(p) => p.id = id,
                }
            }
        }
        if anchors.is_empty() {
            return;
        }
        for l in copies.iter_mut() {
            let stacks = [&mut l.filters]
                .into_iter()
                .chain(l.mask.iter_mut().map(|m| &mut m.filters));
            for stack in stacks {
                for e in stack.iter_mut() {
                    if let EffectSettings::Generator(g) = &mut e.settings {
                        if g.kind == generator::Kind::Anchor {
                            if let Some(to) = anchors.get(&g.anchor.id) {
                                g.anchor.id = *to;
                            }
                        }
                    }
                }
            }
        }
    }

    /// 大きさを変えて写したレイヤー（画素の大きさを変える操作の結果）の、ぼかし・シャープの半径を倍率に合わせる（C# の `Resampled` と同じ:
    /// 半径 × 倍率を 0 から遠い向きへ丸め、1 以上・段の最大以下。変えたものは理由を返す）。段の ID・並び・有効・強さ・チャンネルは
    /// そのまま。縦横の倍率が違うときは、呼び手が幾何平均を渡す。
    pub(crate) fn scale_effect_radii(layers: &mut [Layer], scale: f64) -> Vec<String> {
        let mut notes = Vec::new();
        if scale == 1.0 || !scale.is_finite() || scale <= 0.0 {
            return notes;
        }
        for l in layers.iter_mut() {
            let name = l.name.clone();
            let mut stacks: Vec<(&mut Vec<FilterEffect>, String)> =
                vec![(&mut l.filters, name.clone())];
            if let Some(m) = &mut l.mask {
                stacks.push((&mut m.filters, format!("{name}（マスク）")));
            }
            for (stack, owner) in stacks {
                for e in stack.iter_mut() {
                    let label = e.settings.name();
                    let EffectSettings::Filter(f) = &mut e.settings else {
                        continue;
                    };
                    // 0.5.0 の段の実数の長さ（画素）とノイズの大きさは、倍率を掛けて範囲へ収める
                    let mut real = |v: &mut f64, range: std::ops::RangeInclusive<f64>| {
                        let wanted = *v * scale;
                        let fitted = wanted.clamp(*range.start(), *range.end());
                        if fitted != wanted {
                            notes.push(format!(
                                "「{owner}」の{label}: {} → {fitted} 画素（範囲の端。見た目を保つには {wanted:.1} 画素）",
                                *v
                            ));
                        }
                        *v = fitted;
                    };
                    let (radius, max) = match f {
                        filter::Settings::GaussianBlur { radius } => (radius, 256u32),
                        filter::Settings::Sharpen { radius, .. } => (radius, 64u32),
                        filter::Settings::Morphology { radius, .. } => {
                            (radius, *crate::ranges::MORPHOLOGY_RADIUS.end())
                        }
                        filter::Settings::EdgeDetect { width, .. } => {
                            (width, *crate::ranges::EDGE_WIDTH.end())
                        }
                        filter::Settings::HighPass { radius } => {
                            (radius, *crate::ranges::HIGH_PASS_RADIUS.end())
                        }
                        filter::Settings::Median { radius } => {
                            (radius, *crate::ranges::MEDIAN_RADIUS.end())
                        }
                        filter::Settings::Glow { radius, .. } => {
                            (radius, *crate::ranges::GLOW_RADIUS.end())
                        }
                        filter::Settings::SlopeBlur {
                            intensity, scale, ..
                        } => {
                            real(intensity, crate::ranges::SLOPE_INTENSITY);
                            real(scale, crate::ranges::FILTER_NOISE_SCALE);
                            continue;
                        }
                        filter::Settings::DirectionalBlur { distance, .. } => {
                            real(distance, crate::ranges::DIRECTIONAL_DISTANCE);
                            continue;
                        }
                        filter::Settings::Warp {
                            intensity, scale, ..
                        } => {
                            real(intensity, crate::ranges::WARP_INTENSITY);
                            real(scale, crate::ranges::FILTER_NOISE_SCALE);
                            continue;
                        }
                        _ => continue,
                    };
                    let wanted = f64::from(*radius) * scale;
                    let rounded = wanted.round();
                    let scaled = if rounded > f64::from(max) {
                        notes.push(format!(
                            "「{owner}」の{label}: 半径 {} → {max} 画素（最大。見た目を保つには {wanted:.1} 画素）",
                            *radius
                        ));
                        max
                    } else if rounded < 1.0 {
                        notes.push(format!(
                            "「{owner}」の{label}: 半径 {} → 1 画素（最小。見た目を保つには {wanted:.1} 画素）",
                            *radius
                        ));
                        1
                    } else {
                        rounded as u32
                    };
                    *radius = scaled;
                }
            }
        }
        notes
    }

    /// 文書へ入れる前のレイヤー（読み込み・スマートマテリアルの配置など）の段が、今の文書の決まり（段の数・到達半径・作業メモリ・
    /// ID の重ならなさ・標準のチャンネル）に収まるか。収まらなければ何も変えずに断る。
    pub(crate) fn check_layer_effects(&self, layers: &[Layer]) -> Result<(), CoreError> {
        let mut seen = std::collections::HashSet::new();
        for l in layers {
            for e in l
                .filters
                .iter()
                .chain(l.mask.iter().flat_map(|m| m.filters.iter()))
            {
                if e.id.0 == 0 || !seen.insert(e.id) || self.find_filter(e.id).is_some() {
                    return Err(CoreError::InvalidArgument(
                        "フィルターの ID が空か重なっている",
                    ));
                }
            }
        }
        self.check_layer_stacks(layers)
    }

    /// レイヤーの段が、今の文書の大きさ・予算で収まるか（標準のチャンネル・段の数・到達半径・作業メモリ）。ID は見ない。大きさを変えた写しの
    /// 検査にも使う（同じ ID のレイヤーが文書の中にあってよい）。
    pub(crate) fn check_layer_stacks(&self, layers: &[Layer]) -> Result<(), CoreError> {
        for l in layers {
            for c in l.filters.iter().flat_map(|e| e.channels.iter()) {
                require_standard(*c)?;
            }
            self.check_stack(&l.filters, FilterTarget::Content)?;
            if let Some(m) = &l.mask {
                self.check_stack(&m.filters, FilterTarget::Mask)?;
            }
        }
        Ok(())
    }

    // ───────── フィルターのスタックの編集 ─────────

    /// 段を足せない理由（足せるなら Ok）。内容のスタックはレイヤーの種類（調整・グループにはかけられない）と、設定を受け付けるチャンネル、
    /// マスクのスタックはマスクがあることと、設定が 1 つのスカラーに使えること。
    pub fn filter_refusal(
        &self,
        layer: LayerId,
        target: FilterTarget,
        settings: &EffectSettings,
        channel: Channel,
    ) -> Result<(), CoreError> {
        let index = self.index_of(layer)?;
        match target {
            FilterTarget::Mask => {
                self.stack_ref(index, target)?;
                settings.validate(ValueType::Mask)
            }
            FilterTarget::Content => {
                Self::content_kind_refusal(&self.layers[index])?;
                require_standard(channel)?;
                settings.validate(value_type_of(self.channel_kind(channel)?))
            }
        }
    }

    fn content_kind_refusal(layer: &Layer) -> Result<(), CoreError> {
        match layer.kind {
            LayerKind::Adjustment => Err(CoreError::Unsupported("調整レイヤーには画素が無い")),
            LayerKind::Group => Err(CoreError::Unsupported("グループの合成へのフィルターは無い")),
            _ => Ok(()),
        }
    }

    /// レイヤーのスタックの上（既定）か `spec.index` へ段を足す（1 回の Undo）。内容のスタックは適用するチャンネルを選ぶ
    /// （指定が無ければ設定を受け付ける標準のチャンネル全部）。段の数・到達半径・作業メモリの上限を超えるなら断る。
    pub fn add_filter(
        &mut self,
        layer: LayerId,
        target: FilterTarget,
        spec: FilterSpec,
    ) -> Result<FilterId, CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        require_finite(spec.strength, "strength")?;
        if !(0.0..=1.0).contains(&spec.strength) {
            return Err(CoreError::InvalidArgument("strength"));
        }
        let channels = match target {
            FilterTarget::Content => {
                Self::content_kind_refusal(&self.layers[index])?;
                let mut list: Vec<Channel> = match &spec.channels {
                    Some(given) => {
                        for c in given {
                            self.require_channel(*c)?;
                            require_standard(*c)?;
                            spec.settings
                                .validate(value_type_of(self.channel_kind(*c)?))?;
                        }
                        given.clone()
                    }
                    None => {
                        let mut all = Vec::new();
                        for c in Channel::ALL {
                            if spec
                                .settings
                                .validate(value_type_of(self.channel_kind(c)?))
                                .is_ok()
                            {
                                all.push(c);
                            }
                        }
                        all
                    }
                };
                list.sort();
                if list.windows(2).any(|w| w[0] == w[1]) {
                    return Err(CoreError::InvalidArgument(
                        "フィルターのチャンネルが重なっている",
                    ));
                }
                if list.is_empty() {
                    return Err(CoreError::InvalidArgument(
                        "フィルターのチャンネルが選ばれていない",
                    ));
                }
                list
            }
            FilterTarget::Mask => {
                self.stack_ref(index, target)?;
                if spec.channels.is_some() {
                    return Err(CoreError::InvalidArgument(
                        "マスクのフィルターはチャンネルを持たない（マスクは全チャンネルで共有）",
                    ));
                }
                spec.settings.validate(ValueType::Mask)?;
                Vec::new()
            }
        };
        let id = match spec.id {
            Some(id) => {
                if id.0 == 0 {
                    return Err(CoreError::InvalidArgument("フィルターの ID が空"));
                }
                if self.find_filter(id).is_some() {
                    return Err(CoreError::InvalidArgument("フィルターの ID が重なっている"));
                }
                id
            }
            None => self.new_filter_id(),
        };
        let mut stack = self.stack_ref(index, target)?.clone();
        let at = spec.index.unwrap_or(stack.len());
        if at > stack.len() {
            return Err(CoreError::InvalidArgument("index"));
        }
        stack.insert(
            at,
            FilterEffect {
                id,
                settings: spec.settings,
                enabled: spec.enabled,
                strength: spec.strength,
                channels,
            },
        );
        self.execute_stack(index, target, stack, None)?;
        Ok(id)
    }

    /// 段を外す（1 回の Undo）。
    pub fn remove_filter(&mut self, layer: LayerId, filter: FilterId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let (index, target, at) = self.locate_filter(layer, filter)?;
        let mut stack = self.stack_ref(index, target)?.clone();
        stack.remove(at);
        self.execute_stack(index, target, stack, None)
    }

    /// 段をスタックの中の位置へ動かす（0 が最初に当たる）。
    pub fn move_filter(
        &mut self,
        layer: LayerId,
        filter: FilterId,
        index: usize,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let (layer_index, target, at) = self.locate_filter(layer, filter)?;
        let mut stack = self.stack_ref(layer_index, target)?.clone();
        if index >= stack.len() {
            return Err(CoreError::InvalidArgument("index"));
        }
        if at == index {
            return Ok(());
        }
        let e = stack.remove(at);
        stack.insert(index, e);
        self.execute_stack(layer_index, target, stack, None)
    }

    pub fn set_filter_enabled(
        &mut self,
        layer: LayerId,
        filter: FilterId,
        enabled: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let (index, target, at) = self.locate_filter(layer, filter)?;
        let mut stack = self.stack_ref(index, target)?.clone();
        if stack[at].enabled == enabled {
            return Ok(());
        }
        stack[at].enabled = enabled;
        self.execute_stack(index, target, stack, None)
    }

    /// 段の結果をどれだけ使うか（0〜1）。coalesce ならドラッグの続きをまとめる。
    pub fn set_filter_strength(
        &mut self,
        layer: LayerId,
        filter: FilterId,
        strength: f64,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        require_finite(strength, "strength")?;
        if !(0.0..=1.0).contains(&strength) {
            return Err(CoreError::InvalidArgument("strength"));
        }
        let (index, target, at) = self.locate_filter(layer, filter)?;
        let mut stack = self.stack_ref(index, target)?.clone();
        if stack[at].strength == strength {
            return Ok(());
        }
        stack[at].strength = strength;
        self.execute_stack(
            index,
            target,
            stack,
            coalesce.then_some(CoalesceKey::FilterStrength(filter)),
        )
    }

    /// 段の設定を置き換える（種類も変えられる。適用するチャンネルのどれもが新しい設定を受け付けるときだけ）。coalesce ならドラッグをまとめる。
    pub fn set_filter_settings(
        &mut self,
        layer: LayerId,
        filter: FilterId,
        settings: EffectSettings,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let (index, target, at) = self.locate_filter(layer, filter)?;
        match target {
            FilterTarget::Mask => settings.validate(ValueType::Mask)?,
            FilterTarget::Content => {
                for c in &self.stack_ref(index, target)?[at].channels {
                    settings.validate(value_type_of(self.channel_kind(*c)?))?;
                }
            }
        }
        let mut stack = self.stack_ref(index, target)?.clone();
        if stack[at].settings == settings {
            return Ok(());
        }
        stack[at].settings = settings;
        self.execute_stack(
            index,
            target,
            stack,
            coalesce.then_some(CoalesceKey::FilterSettings(filter)),
        )
    }

    /// 内容の段が適用するチャンネルを置き換える（どれも設定を受け付けること）。
    pub fn set_filter_channels(
        &mut self,
        layer: LayerId,
        filter: FilterId,
        channels: &[Channel],
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let (index, target, at) = self.locate_filter(layer, filter)?;
        if target == FilterTarget::Mask {
            return Err(CoreError::Unsupported(
                "マスクのフィルターはチャンネルを持たない",
            ));
        }
        let mut list = channels.to_vec();
        list.sort();
        list.dedup();
        if list.is_empty() {
            return Err(CoreError::InvalidArgument(
                "フィルターのチャンネルが選ばれていない",
            ));
        }
        let mut stack = self.stack_ref(index, target)?.clone();
        for c in &list {
            self.require_channel(*c)?;
            require_standard(*c)?;
            stack[at]
                .settings
                .validate(value_type_of(self.channel_kind(*c)?))?;
        }
        if stack[at].channels == list {
            return Ok(());
        }
        stack[at].channels = list;
        self.execute_stack(index, target, stack, None)
    }

    fn locate_filter(
        &self,
        layer: LayerId,
        filter: FilterId,
    ) -> Result<(usize, FilterTarget, usize), CoreError> {
        let index = self.index_of(layer)?;
        let l = &self.layers[index];
        if let Some(at) = l.filters.iter().position(|e| e.id == filter) {
            return Ok((index, FilterTarget::Content, at));
        }
        if let Some(at) = l
            .mask
            .as_ref()
            .and_then(|m| m.filters.iter().position(|e| e.id == filter))
        {
            return Ok((index, FilterTarget::Mask, at));
        }
        Err(CoreError::Unsupported("そのレイヤーにそのフィルターが無い"))
    }

    /// スタックの検査（段の数・到達半径・ブロックの作業メモリ）。チャンネルごと・マスクの有効な段の並びで見る。
    fn check_stack(&self, after: &[FilterEffect], target: FilterTarget) -> Result<(), CoreError> {
        if after.len() > MAX_FILTERS_PER_STACK {
            return Err(CoreError::Unsupported("1 つのスタックの段は 32 まで"));
        }
        let chains: Vec<Vec<&FilterEffect>> = match target {
            FilterTarget::Mask => vec![after.iter().filter(|e| e.is_active()).collect()],
            FilterTarget::Content => Channel::ALL
                .iter()
                .map(|c| {
                    after
                        .iter()
                        .filter(|e| e.is_active() && e.applies_to(*c))
                        .collect()
                })
                .collect(),
        };
        for chain in &chains {
            self.check_chain(chain)?;
        }
        Ok(())
    }

    fn check_chain(&self, chain: &[&FilterEffect]) -> Result<(), CoreError> {
        let halo: u32 = chain.iter().map(|e| e.settings.halo()).sum();
        if halo > MAX_FILTER_STACK_HALO {
            return Err(CoreError::Unsupported(
                "スタックの到達半径の合計が 512 画素を超える",
            ));
        }
        if chain.is_empty() {
            return Ok(());
        }
        let need = self.block_need_bytes(chain)?;
        if need > self.effects.working_budget {
            return Err(CoreError::WorkingBudgetExceeded);
        }
        Ok(())
    }

    /// 1 ブロックの評価が使う作業バイト数（フィルターの作業と、返す画像）。
    pub(crate) fn block_need_bytes(&self, chain: &[&FilterEffect]) -> Result<u64, CoreError> {
        self.block_need_with(chain, self.effects.block_pixels)
    }

    fn block_need_with(&self, chain: &[&FilterEffect], block: u32) -> Result<u64, CoreError> {
        let side = (block / self.tile_size).max(1) * self.tile_size;
        let (w, h) = (side.min(self.width), side.min(self.height));
        let stages = stages_of(chain);
        let mut working = filter::block_working_bytes(&stages, block, self.width, self.height)
            .map_err(|e| match e {
                filter::Error::Invalid(why) => CoreError::InvalidArgument(why),
                _ => CoreError::WorkingBudgetExceeded,
            })?;
        // 継ぎ目をまたいで評価するなら、その分も（`filter::evaluate` が足すのと同じ見積り）
        if self.seam_shape(chain.iter().map(|e| e.settings.halo())).0 > 0 {
            // 帯の写しを作る前なので、帯のテクセルの数は分からない（最悪: 入力の画素ぜんぶ）。評価（`filter::evaluate`）は帯の写しの実際の数で
            // 見積もるので、これより小さい
            working = working.saturating_add(filter::seam_working_bytes(
                &stages,
                block,
                self.width,
                self.height,
                u64::MAX,
            ));
        }
        Ok(working.saturating_add(u64::from(w) * u64::from(h) * 4))
    }

    /// 新しいスタックを検査して、1 つの Undo として入れ替える。レイヤーが出す所（前・後）を作り直させる。
    fn execute_stack(
        &mut self,
        index: usize,
        target: FilterTarget,
        after: Vec<FilterEffect>,
        key: Option<CoalesceKey>,
    ) -> Result<(), CoreError> {
        // 効果は非破壊（画素は変えない）ので、画像のロックでは足せる・動かせる。すべてのロックだけで断る（C# の ExecuteStack。
        // 検査の中でいちばん先: 段の数・到達半径・作業メモリより前）
        self.refuse_lock(self.layers[index].id, super::LayerLocks::ALL)?;
        self.check_stack(&after, target)?;
        let before = self.stack_ref(index, target)?.clone();
        let ramp = |s: &[FilterEffect]| -> u64 {
            let generators = s
                .iter()
                .filter(|e| matches!(&e.settings, EffectSettings::Generator(g) if g.ramp.is_some()))
                .count() as u64
                * 512;
            // 色調補正の段（ランプ・曲線・表）は、その中身の大きさ
            let adjustments: u64 = s
                .iter()
                .filter_map(|e| e.settings.color_adjust())
                .map(|a| a.byte_size())
                .sum();
            generators + adjustments
        };
        let cost = 64 + 96 * (before.len() + after.len()) as u64 + ramp(&before) + ramp(&after);
        let id = self.layers[index].id;
        self.record(
            Command::Stack {
                id,
                target,
                before,
                after,
            },
            cost,
            key,
        )
    }

    /// 段の入れ替え（Undo・Redo・最初の実行）。前後のレイヤーが出す所に印を付ける。
    pub(super) fn switch_stack(
        &mut self,
        id: LayerId,
        target: FilterTarget,
        stack: &[FilterEffect],
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        self.mark_layer(index, None);
        match target {
            FilterTarget::Content => self.layers[index].filters = stack.to_vec(),
            FilterTarget::Mask => {
                self.layers[index]
                    .mask
                    .as_mut()
                    .ok_or(CoreError::Unsupported("レイヤーにマスクが無い"))?
                    .filters = stack.to_vec()
            }
        }
        self.mark_layer(index, None);
        self.mark_clipped_layers();
        Ok(())
    }

    // ───────── Anchor ─────────

    /// レイヤーの Anchor（下から上。レイヤーの Anchor、次にそのマスクの Anchor）。
    pub fn anchors(&self) -> Vec<AnchorInfo<'_>> {
        let mut v = Vec::new();
        for l in &self.layers {
            if let Some(a) = &l.anchor {
                v.push(AnchorInfo {
                    anchor: a,
                    layer: l.id,
                    placement: AnchorPlacement::Layer,
                });
            }
            if let Some(a) = l.mask.as_ref().and_then(|m| m.anchor.as_ref()) {
                v.push(AnchorInfo {
                    anchor: a,
                    layer: l.id,
                    placement: AnchorPlacement::Mask,
                });
            }
        }
        v
    }

    pub fn find_anchor(&self, id: AnchorId) -> Option<AnchorInfo<'_>> {
        self.anchors().into_iter().find(|a| a.anchor.id == id)
    }

    /// レイヤー（またはそのマスク）に Anchor を置く（1 回の Undo）。レイヤーごとに、置き場ごとに 1 つまで。マスクの Anchor はマスクが要る。
    /// 名前の既定はレイヤーの名前（マスクは「（マスク）」を付ける）。
    pub fn add_anchor(
        &mut self,
        layer: LayerId,
        placement: AnchorPlacement,
        name: Option<&str>,
        id: Option<AnchorId>,
    ) -> Result<AnchorId, CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        let l = &self.layers[index];
        match placement {
            AnchorPlacement::Mask => {
                let m = l.mask.as_ref().ok_or(CoreError::Unsupported(
                    "マスクの無いレイヤーのマスクには Anchor を置けない",
                ))?;
                if m.anchor.is_some() {
                    return Err(CoreError::Unsupported("このマスクにはもう Anchor がある"));
                }
            }
            AnchorPlacement::Layer => {
                if l.anchor.is_some() {
                    return Err(CoreError::Unsupported("このレイヤーにはもう Anchor がある"));
                }
            }
        }
        // 画素は変えないので、すべてのロックだけで断る（フィルターと同じ。C# の AddAnchor は、置き場の検査のあと・ID と名前の検査の前）
        self.refuse_lock(layer, super::LayerLocks::ALL)?;
        let name = match name {
            Some(n) => n.to_owned(),
            None if placement == AnchorPlacement::Mask => format!("{}（マスク）", l.name),
            None => l.name.clone(),
        };
        Anchor::check_name(&name)?;
        let anchor_id = match id {
            Some(a) => {
                if a.0 == 0 {
                    return Err(CoreError::InvalidArgument("Anchor の ID が空"));
                }
                if self.find_anchor(a).is_some() {
                    return Err(CoreError::InvalidArgument("Anchor の ID が重なっている"));
                }
                a
            }
            None => self.new_anchor_id(),
        };
        // 履歴の費用は名前の長さによらず一定（C# の AddAnchor は 64。名前の変更だけが長さを数える）
        let cost = 64;
        self.execute(
            Command::Anchor {
                id: layer,
                placement,
                old: None,
                new: Some(Anchor {
                    id: anchor_id,
                    name,
                }),
            },
            cost,
        )?;
        Ok(anchor_id)
    }

    /// Anchor を外す（1 回の Undo）。読んでいる Generator は入力のまま通し、理由を出す（別の Anchor を選ぶか Undo するまで）。
    pub fn remove_anchor(&mut self, anchor: AnchorId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let info = self
            .find_anchor(anchor)
            .ok_or(CoreError::Unsupported("その Anchor が無い"))?;
        let (layer, placement, old) = (info.layer, info.placement, info.anchor.clone());
        // 名前の変更は断らないが、外すのは、すべてのロックで断る（C# の RemoveAnchor）
        self.refuse_lock(layer, super::LayerLocks::ALL)?;
        let cost = 64;
        self.execute(
            Command::Anchor {
                id: layer,
                placement,
                old: Some(old),
                new: None,
            },
            cost,
        )
    }

    pub fn rename_anchor(&mut self, anchor: AnchorId, name: &str) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        Anchor::check_name(name)?;
        let info = self
            .find_anchor(anchor)
            .ok_or(CoreError::Unsupported("その Anchor が無い"))?;
        if info.anchor.name == name {
            return Ok(());
        }
        let (layer, placement, old) = (info.layer, info.placement, info.anchor.clone());
        let cost = 64 + 2 * (old.name.encode_utf16().count() + name.encode_utf16().count()) as u64;
        self.execute(
            Command::Anchor {
                id: layer,
                placement,
                old: Some(old),
                new: Some(Anchor {
                    id: anchor,
                    name: name.to_owned(),
                }),
            },
            cost,
        )
    }

    pub(super) fn switch_anchor(
        &mut self,
        id: LayerId,
        placement: AnchorPlacement,
        value: Option<&Anchor>,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        match placement {
            AnchorPlacement::Layer => self.layers[index].anchor = value.cloned(),
            AnchorPlacement::Mask => {
                self.layers[index]
                    .mask
                    .as_mut()
                    .ok_or(CoreError::Unsupported("レイヤーにマスクが無い"))?
                    .anchor = value.cloned()
            }
        }
        // 名前だけの変更・置く・外すは合成を変えない。読む側の解決は `refresh_anchor_readers` が見る
        Ok(())
    }

    /// レイヤーの上の Generator が読める Anchor かの確かめ（読めるなら None）。選ばない（None）は断らない。
    pub fn anchor_reference_refusal(
        &self,
        layer: LayerId,
        anchor: Option<AnchorId>,
    ) -> Result<Option<AnchorIssueKind>, CoreError> {
        let reader = self.index_of(layer)?;
        let Some(a) = anchor else {
            return Ok(None);
        };
        let points = self.anchor_points();
        Ok(
            match anchor::resolve(&points, a.0, reader, self.layers.len()) {
                Ok(_) => None,
                Err(anchor::Issue::NotChosen) => Some(AnchorIssueKind::NotChosen),
                Err(anchor::Issue::Missing) => Some(AnchorIssueKind::Missing),
                Err(anchor::Issue::NotBelow) => Some(AnchorIssueKind::NotBelow),
            },
        )
    }

    /// レイヤーの Generator が読める Anchor（そのレイヤーより下にあるもの）。
    pub fn anchors_readable_from(&self, layer: LayerId) -> Result<Vec<AnchorInfo<'_>>, CoreError> {
        self.index_of(layer)?;
        let mut v = Vec::new();
        for info in self.anchors() {
            if self
                .anchor_reference_refusal(layer, Some(info.anchor.id))?
                .is_none()
            {
                v.push(info);
            }
        }
        Ok(v)
    }

    /// Anchor の Generator が読むものを選ぶ（1 回の Undo。coalesce ならまとめる）。読めない Anchor・Normal のチャンネルは断る。
    /// None は何も読まない（入力のまま通す）。
    pub fn set_generator_anchor(
        &mut self,
        layer: LayerId,
        filter: FilterId,
        anchor: Option<AnchorId>,
        channel: Channel,
        read: anchor::ReadMode,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let (index, target, at) = self.locate_filter(layer, filter)?;
        let effect = &self.stack_ref(index, target)?[at];
        let EffectSettings::Generator(g) = &effect.settings else {
            return Err(CoreError::Unsupported("Anchor のジェネレーターではない"));
        };
        if g.kind != generator::Kind::Anchor {
            return Err(CoreError::Unsupported("Anchor のジェネレーターではない"));
        }
        require_standard(channel)?;
        if channel == Channel::Normal {
            return Err(CoreError::Unsupported("Anchor は Normal を読めない"));
        }
        if let Some(why) = self.anchor_reference_refusal(layer, anchor)? {
            return Err(CoreError::Unsupported(match why {
                AnchorIssueKind::NotBelow => {
                    "Anchor がレイヤーより下に無い（自分のレイヤーの Anchor は読めない）"
                }
                _ => "その Anchor が無い",
            }));
        }
        let mut next = (**g).clone();
        next.anchor = anchor::Reference {
            id: anchor.map_or(0, |a| a.0),
            channel,
            read,
        };
        self.set_filter_settings(layer, filter, EffectSettings::generator(next), coalesce)
    }

    /// 入力のまま通している Anchor の Generator（読む Anchor を選んでいない・無い・レイヤーより下に無い）。下から上の順。
    pub fn anchor_issues(&self) -> Vec<AnchorIssue> {
        let points = self.anchor_points();
        let mut v = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let stacks = [
                (FilterTarget::Content, l.filters.as_slice()),
                (
                    FilterTarget::Mask,
                    l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
                ),
            ];
            for (target, stack) in stacks {
                for e in stack {
                    let EffectSettings::Generator(g) = &e.settings else {
                        continue;
                    };
                    if g.kind != generator::Kind::Anchor {
                        continue;
                    }
                    if let Err(issue) = g.anchor.resolve(&points, i, self.layers.len()) {
                        v.push(AnchorIssue {
                            layer: l.id,
                            filter: e.id,
                            target,
                            anchor: (g.anchor.id != 0).then_some(AnchorId(g.anchor.id)),
                            kind: match issue {
                                anchor::Issue::NotChosen => AnchorIssueKind::NotChosen,
                                anchor::Issue::Missing => AnchorIssueKind::Missing,
                                anchor::Issue::NotBelow => AnchorIssueKind::NotBelow,
                            },
                        });
                    }
                }
            }
        }
        v
    }

    /// 文書の Anchor を、評価器の点（レイヤーの番号と置き場）の並びにする。
    pub(crate) fn anchor_points(&self) -> Vec<anchor::Point> {
        let mut v = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            if let Some(a) = &l.anchor {
                v.push(anchor::Point {
                    id: a.id.0,
                    host: i,
                    placement: anchor::Placement::Layer,
                });
            }
            if let Some(a) = l.mask.as_ref().and_then(|m| m.anchor.as_ref()) {
                v.push(anchor::Point {
                    id: a.id.0,
                    host: i,
                    placement: anchor::Placement::Mask,
                });
            }
        }
        v
    }

    /// 読み込み用: 履歴なしでレイヤー・マスクに Anchor を置く（検査は編集と同じ。読み込みなので履歴を消す）。
    pub fn set_anchor_for_load(
        &mut self,
        layer: LayerId,
        placement: AnchorPlacement,
        id: AnchorId,
        name: &str,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        Anchor::check_name(name)?;
        if id.0 == 0 || self.find_anchor(id).is_some() {
            return Err(CoreError::InvalidArgument(
                "Anchor の ID が空か重なっている",
            ));
        }
        let anchor = Anchor {
            id,
            name: name.to_owned(),
        };
        match placement {
            AnchorPlacement::Layer => {
                if self.layers[index].anchor.is_some() {
                    return Err(CoreError::Unsupported("このレイヤーにはもう Anchor がある"));
                }
                self.layers[index].anchor = Some(anchor);
            }
            AnchorPlacement::Mask => {
                let m = self.layers[index]
                    .mask
                    .as_mut()
                    .ok_or(CoreError::Unsupported("マスクが無い"))?;
                if m.anchor.is_some() {
                    return Err(CoreError::Unsupported("このマスクにはもう Anchor がある"));
                }
                m.anchor = Some(anchor);
            }
        }
        self.external_mutation();
        Ok(())
    }

    /// 読み込みの最後に: 自分のレイヤーの Anchor を読む段（値が自分に戻る参照。どの編集でも作れない）があれば断る。消えた Anchor・上のレイヤーの
    /// Anchor を指す参照は、保存されたまま読み込む（入力のまま通し、保存しても参照は残る）。
    pub fn check_anchor_references_for_load(&self) -> Result<(), CoreError> {
        let points = self.anchor_points();
        for (i, l) in self.layers.iter().enumerate() {
            let stacks = [
                l.filters.as_slice(),
                l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
            ];
            for stack in stacks {
                for e in stack {
                    if let EffectSettings::Generator(g) = &e.settings {
                        if g.kind == generator::Kind::Anchor {
                            if let Some(p) = points.iter().find(|p| p.id == g.anchor.id) {
                                if p.host == i && g.anchor.id != 0 {
                                    return Err(CoreError::InvalidArgument(
                                        "自分のレイヤーの Anchor を読むジェネレーター（値が自分に戻る）",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    // ───────── 塗りつぶしの画像・投影・グラデーション ─────────

    fn fill_layer_index(&self, layer: LayerId) -> Result<usize, CoreError> {
        let index = self.index_of(layer)?;
        if self.layers[index].kind != LayerKind::Fill {
            return Err(CoreError::Unsupported("塗りつぶしレイヤーだけが持つ"));
        }
        Ok(index)
    }

    /// 塗りつぶしのチャンネルが読む画像を置く（None で外す。1 回の Undo）。値の無いチャンネルに置くと、既定の値
    /// （画像が使えない所に出る）を持ち、チャンネルを有効にする。外しても値は残る。置くとそのチャンネルのグラデーションは外れる
    /// （Undo で戻る）。渡した入力に無い画像・ミップマップが画像の予算を超える画像は断る。
    pub fn set_fill_image(
        &mut self,
        layer: LayerId,
        channel: Channel,
        image: Option<ImageId>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let kind = self.channel_kind(channel)?;
        require_standard(channel)?;
        let index = self.fill_layer_index(layer)?;
        let old = FillChannelState::of(&self.layers[index], channel);
        if old.image == image {
            return Ok(());
        }
        if let Some(id) = image {
            if id.0 == 0 {
                return Err(CoreError::InvalidArgument("画像の ID が空"));
            }
            let input = self
                .effects
                .inputs
                .images
                .get(&id)
                .ok_or(CoreError::Unsupported("プロジェクトにその画像が無い"))?;
            let need = ImageMipChain::extra_bytes(input.width, input.height)
                .map_err(|_| CoreError::InvalidArgument("画像の大きさ"))?;
            if need > self.effects.image_cache_budget {
                return Err(CoreError::WorkingBudgetExceeded);
            }
        }
        // 画像は場所ごとに不透明度が変わるので、画像・透明部分・すべてのロックで断る（C# の SetFillImage。入力の検査のあと）
        self.ensure_pixels_rewritable(layer)?;
        let mut new = old.clone();
        new.image = image;
        if image.is_none() {
            // 画像の読み方は画像と一緒に保存する（画像の無いチャンネルは既定に戻す）
            new.anisotropic = true;
        }
        if image.is_some() {
            new.gradient = None;
            new.points = None;
            if new.value.is_none() {
                new.value = Some(default_fill_fallback(kind));
            }
            new.enabled = true;
        }
        self.execute(
            Command::FillChannel {
                id: layer,
                channel,
                old: Box::new(old),
                new: Box::new(new),
            },
            96,
        )
    }

    /// 塗りつぶしのチャンネルの画像を異方性のフィルターで読むか（1 回の Undo）。画像の無いチャンネルは断る。
    pub fn set_fill_anisotropic(
        &mut self,
        layer: LayerId,
        channel: Channel,
        anisotropic: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        require_standard(channel)?;
        let index = self.fill_layer_index(layer)?;
        let old = FillChannelState::of(&self.layers[index], channel);
        if old.image.is_none() {
            return Err(CoreError::InvalidArgument("画像の無いチャンネル"));
        }
        if old.anisotropic == anisotropic {
            return Ok(());
        }
        self.ensure_pixels_rewritable(layer)?;
        let mut new = old.clone();
        new.anisotropic = anisotropic;
        self.execute(
            Command::FillChannel {
                id: layer,
                channel,
                old: Box::new(old),
                new: Box::new(new),
            },
            96,
        )
    }

    /// 読み込み用: 履歴なしで、塗りつぶしの画像を異方性のフィルターなしで読むチャンネルを置く（画像のあるチャンネルだけ）。
    pub fn set_fill_isotropic_for_load(
        &mut self,
        layer: LayerId,
        channels: &[Channel],
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.fill_layer_index(layer)?;
        let mut set = std::collections::BTreeSet::new();
        for c in channels {
            if !self.layers[index].fill_images.contains_key(c) || !set.insert(*c) {
                return Err(CoreError::InvalidArgument(
                    "異方性を切る塗りつぶしの画像のチャンネル",
                ));
            }
        }
        self.layers[index].fill_isotropic = set;
        self.external_mutation();
        Ok(())
    }

    /// 塗りつぶしの投影（レイヤーで 1 つ。UV・トライプラナー・平面・球・円柱・デカール）を置き換える（1 回の Undo。coalesce ならドラッグをまとめる）。
    pub fn set_fill_projection(
        &mut self,
        layer: LayerId,
        projection: Projection,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.fill_layer_index(layer)?;
        projection
            .validate()
            .map_err(|_| CoreError::InvalidArgument("投影の値"))?;
        let old = self.layers[index].projection;
        if old == projection {
            return Ok(());
        }
        // 画像のあるレイヤー・デカール（前か後）は画素を変えるので、画像・透明部分のロックでも断る。それ以外は、すべてのロックだけ
        // （C# の SetFillProjection）
        if !self.layers[index].fill_images.is_empty()
            || old.mode == ProjectionMode::Decal
            || projection.mode == ProjectionMode::Decal
        {
            self.ensure_pixels_rewritable(layer)?;
        } else {
            self.refuse_lock(layer, super::LayerLocks::ALL)?;
        }
        self.record(
            Command::Projection {
                id: layer,
                old,
                new: projection,
            },
            160,
            coalesce.then_some(CoalesceKey::Projection(layer)),
        )
    }

    /// 塗りつぶしのチャンネルのグラデーション（ランプ付きの形のグラデーションの Generator。置き換え）を置く（None で外す。1 回の Undo。
    /// coalesce ならドラッグをまとめる）。置くとそのチャンネルの画像は外れる（Undo で戻る）。マップが使えない間は値を見せる。
    pub fn set_fill_gradient(
        &mut self,
        layer: LayerId,
        channel: Channel,
        gradient: Option<generator::Settings>,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let kind = self.channel_kind(channel)?;
        require_standard(channel)?;
        let index = self.fill_layer_index(layer)?;
        if let Some(g) = &gradient {
            validate_fill_gradient(channel, g)?;
        }
        let old = FillChannelState::of(&self.layers[index], channel);
        if old.gradient == gradient {
            return Ok(());
        }
        // グラデーションも画素を決め直す（アルファも変わる）ので、画像・透明部分・すべてのロックで断る（C# の SetFillGradient）
        self.ensure_pixels_rewritable(layer)?;
        // 履歴の費用は C# の SetFillGradient と同じ（96 + 前後のランプの大きさ）
        let ramp_bytes = |g: &Option<generator::Settings>| {
            g.as_ref()
                .and_then(|g| g.ramp.as_ref())
                .map_or(0, generator::Ramp::byte_size)
        };
        let cost = 96 + ramp_bytes(&old.gradient) + ramp_bytes(&gradient);
        let mut new = old.clone();
        new.gradient = gradient;
        if new.gradient.is_some() {
            new.image = None;
            new.points = None;
            if new.value.is_none() {
                new.value = Some(default_fill_fallback(kind));
            }
            new.enabled = true;
        }
        self.record(
            Command::FillChannel {
                id: layer,
                channel,
                old: Box::new(old),
                new: Box::new(new),
            },
            cost,
            coalesce.then_some(CoalesceKey::FillGradient(layer, channel)),
        )
    }

    /// 塗りつぶしのチャンネルの点のグラデーションを置き換える・外す（1 回の Undo。coalesce なら点のドラッグをまとめる）。置くと、その
    /// チャンネルの画像・形のグラデーションは外れる。法線のチャンネル・点の数（1〜64）・位置・広がりの範囲の外は断る。
    pub fn set_fill_points(
        &mut self,
        layer: LayerId,
        channel: Channel,
        points: Option<PointGradient>,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let kind = self.channel_kind(channel)?;
        require_standard(channel)?;
        let index = self.fill_layer_index(layer)?;
        if let Some(g) = &points {
            validate_fill_points(channel, g)?;
        }
        let old = FillChannelState::of(&self.layers[index], channel);
        if old.points == points {
            return Ok(());
        }
        self.ensure_pixels_rewritable(layer)?;
        let cost = 96
            + old
                .points
                .as_ref()
                .map_or(0, |g| g.points.len() as u64 * 32)
            + points.as_ref().map_or(0, |g| g.points.len() as u64 * 32);
        let mut new = old.clone();
        new.points = points;
        if new.points.is_some() {
            new.image = None;
            new.anisotropic = true;
            new.gradient = None;
            if new.value.is_none() {
                new.value = Some(default_fill_fallback(kind));
            }
            new.enabled = true;
        }
        self.record(
            Command::FillChannel {
                id: layer,
                channel,
                old: Box::new(old),
                new: Box::new(new),
            },
            cost,
            coalesce.then_some(CoalesceKey::FillPoints(layer, channel)),
        )
    }

    /// 読み込み用: 履歴なしで塗りつぶしの点のグラデーションを置く（画像・形のグラデーションのあるチャンネルは断る）。
    pub fn set_fill_points_for_load(
        &mut self,
        layer: LayerId,
        points: Vec<(Channel, PointGradient)>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.fill_layer_index(layer)?;
        let mut map = BTreeMap::new();
        for (c, g) in points {
            require_standard(c)?;
            validate_fill_points(c, &g)?;
            let l = &self.layers[index];
            if !l.fill.contains_key(&c)
                || l.fill_images.contains_key(&c)
                || l.fill_gradients.contains_key(&c)
                || map.insert(c, g).is_some()
            {
                return Err(CoreError::InvalidArgument(
                    "塗りつぶしの点のグラデーションのチャンネル",
                ));
            }
        }
        self.layers[index].fill_points = map;
        self.external_mutation();
        Ok(())
    }

    /// 塗りつぶしのチャンネルの点のグラデーションが今は値を見せているなら、その理由（モデルの空間で、位置のマップ・ルートが使えない）。
    pub fn fill_points_inactive(
        &self,
        layer: LayerId,
        channel: Channel,
    ) -> Result<Option<InactiveReason>, CoreError> {
        let index = self.fill_layer_index(layer)?;
        let g = self.layers[index]
            .fill_points
            .get(&channel)
            .ok_or(CoreError::Unsupported(
                "そのチャンネルに点のグラデーションが無い",
            ))?;
        Ok(self.points_reason(g))
    }

    /// 文書の画素 (x, y) の、モデルのルートの空間の位置（効果の入力の位置のマップが今の物で、その画素を覆っているときだけ）。
    pub fn model_position_at(&self, x: u32, y: u32) -> Option<[f64; 3]> {
        let inputs = &self.effects.inputs;
        let map = inputs
            .map(generator::MapKind::Position)
            .filter(|m| m.state == generator::MapState::Current)?;
        if (map.width, map.height) != (self.width, self.height) {
            return None;
        }
        let frame = inputs.frame.and_then(|f| f.for_generator().ok())?;
        crate::fill_points::root_position(&map.as_generator(), frame, x, y)
    }

    fn points_reason(&self, g: &PointGradient) -> Option<InactiveReason> {
        if g.space != PointSpace::Model {
            return None;
        }
        let inputs = &self.effects.inputs;
        let map = inputs
            .map(generator::MapKind::Position)
            .map(|m| m.as_generator());
        let frame = inputs.frame.and_then(|f| f.for_generator().ok());
        crate::fill_points::inactive_reason(g, map.as_ref(), frame, (self.width, self.height))
            .map(InactiveReason::Generator)
    }

    pub(super) fn switch_fill_channel(
        &mut self,
        id: LayerId,
        channel: Channel,
        state: &FillChannelState,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        state.put(&mut self.layers[index], channel);
        self.mark_layer(index, Some(channel));
        self.mark_clipped_layers();
        Ok(())
    }

    pub(super) fn switch_projection(
        &mut self,
        id: LayerId,
        projection: Projection,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        self.layers[index].projection = projection;
        self.mark_layer(index, None);
        self.mark_clipped_layers();
        Ok(())
    }

    /// 読み込み用: 履歴なしで塗りつぶしの画像と投影を置く。画像の ID がプロジェクトに無くても断らない（値を見せ、ID は残る）。
    pub fn set_fill_images_for_load(
        &mut self,
        layer: LayerId,
        images: &[(Channel, ImageId)],
        projection: Projection,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.fill_layer_index(layer)?;
        projection
            .validate()
            .map_err(|_| CoreError::InvalidArgument("投影の値"))?;
        let mut seen = BTreeMap::new();
        for (c, id) in images {
            require_standard(*c)?;
            if id.0 == 0
                || seen.insert(*c, *id).is_some()
                || !self.layers[index].fill.contains_key(c)
                || self.layers[index].fill_gradients.contains_key(c)
                || self.layers[index].fill_points.contains_key(c)
            {
                return Err(CoreError::InvalidArgument(
                    "塗りつぶしの画像のチャンネルか ID",
                ));
            }
        }
        let l = &mut self.layers[index];
        l.fill_isotropic.retain(|c| seen.contains_key(c));
        l.fill_images = seen;
        l.projection = projection;
        self.external_mutation();
        Ok(())
    }

    /// 読み込み用: 履歴なしで塗りつぶしのグラデーションを置く。
    pub fn set_fill_gradients_for_load(
        &mut self,
        layer: LayerId,
        gradients: Vec<(Channel, generator::Settings)>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.fill_layer_index(layer)?;
        let mut map = BTreeMap::new();
        for (c, g) in gradients {
            require_standard(c)?;
            validate_fill_gradient(c, &g)?;
            if !self.layers[index].fill.contains_key(&c)
                || self.layers[index].fill_images.contains_key(&c)
                || map.insert(c, g).is_some()
            {
                return Err(CoreError::InvalidArgument(
                    "塗りつぶしのグラデーションのチャンネル",
                ));
            }
        }
        self.layers[index].fill_gradients = map;
        self.external_mutation();
        Ok(())
    }

    /// 読み込み用: 履歴なしでスタックをそのまま置く（段の検査は編集と同じ）。ID が文書の中で重なるものは断る。
    pub fn set_filters_for_load(
        &mut self,
        layer: LayerId,
        target: FilterTarget,
        specs: Vec<FilterSpec>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        let mut stack = Vec::with_capacity(specs.len());
        let mut kinds_ok = Ok(());
        for spec in specs {
            if let FilterTarget::Content = target {
                kinds_ok = kinds_ok.and(Self::content_kind_refusal(&self.layers[index]));
            }
            let id = spec
                .id
                .ok_or(CoreError::InvalidArgument("フィルターの ID"))?;
            if id.0 == 0
                || self.find_filter(id).is_some()
                || stack.iter().any(|e: &FilterEffect| e.id == id)
            {
                return Err(CoreError::InvalidArgument(
                    "フィルターの ID が空か重なっている",
                ));
            }
            require_finite(spec.strength, "strength")?;
            if !(0.0..=1.0).contains(&spec.strength) {
                return Err(CoreError::InvalidArgument("strength"));
            }
            let channels = match target {
                FilterTarget::Content => {
                    let mut list = spec
                        .channels
                        .ok_or(CoreError::InvalidArgument("フィルターのチャンネル"))?;
                    list.sort();
                    if list.is_empty() || list.windows(2).any(|w| w[0] == w[1]) {
                        return Err(CoreError::InvalidArgument("フィルターのチャンネル"));
                    }
                    for c in &list {
                        self.require_channel(*c)?;
                        require_standard(*c)?;
                        spec.settings
                            .validate(value_type_of(self.channel_kind(*c)?))?;
                    }
                    list
                }
                FilterTarget::Mask => {
                    if spec.channels.is_some() {
                        return Err(CoreError::InvalidArgument("マスクのフィルターのチャンネル"));
                    }
                    spec.settings.validate(ValueType::Mask)?;
                    Vec::new()
                }
            };
            stack.push(FilterEffect {
                id,
                settings: spec.settings,
                enabled: spec.enabled,
                strength: spec.strength,
                channels,
            });
        }
        kinds_ok?;
        if matches!(target, FilterTarget::Mask) {
            self.stack_ref(index, target)?;
        }
        self.check_stack(&stack, target)?;
        match target {
            FilterTarget::Content => self.layers[index].filters = stack,
            FilterTarget::Mask => {
                self.layers[index].mask.as_mut().expect("確かめた").filters = stack
            }
        }
        self.external_mutation();
        Ok(())
    }

    // ───────── 外から渡す入力 ─────────

    /// 文書の外から渡す入力（焼いたメッシュマップ・モデルのルートの位置・プロジェクトの画像・モデルの UV の位相）を置く。保存も Undo もしない。
    /// 今までと違えば、それを読むレイヤーの合成を作り直させる。マップ・画像・モデルのルートが替われば、それを読むレイヤー（Generator・画像・デカール・
    /// グラデーション）。UV の位相が別の物に替われば、継ぎ目をまたぐレイヤー（近傍の段のあるレイヤー）と アイランドごとのばらつきの段のあるレイヤー。
    /// ほかの読み手は評価し直さない。
    pub fn set_effect_inputs(&mut self, mut inputs: EffectInputs) -> Result<(), CoreError> {
        for m in &inputs.maps {
            m.validate()?;
        }
        let topology = !self.effects.inputs.same_topology(&inputs);
        let data = !self.effects.inputs.same_data(&inputs);
        if !topology && !data {
            return Ok(());
        }
        if topology {
            // UV の位相が替わると、継ぎ目をまたぐレイヤーの出力も変わる（前と後の両方で、またぐ所に印を付ける）
            self.mark_seam_readers();
        } else {
            // 同じ UV の位相なら今の物を使い続ける（覚えたアイランドの図・帯の写しを捨てない）
            inputs.topology = self.effects.inputs.topology.clone();
        }
        self.effects.inputs = inputs;
        if topology {
            self.effects.topology_revision += 1;
        }
        if data {
            self.effects.inputs_revision += 1;
            self.mark_input_readers();
        }
        if topology {
            self.mark_seam_readers();
            self.mark_island_readers();
        }
        Ok(())
    }

    /// アイランドごとのばらつきの段のあるレイヤー（内容・マスク）が出す所に印を付ける（モデルの UV の位相・アイランドの図の予算が替わったとき）。
    pub(super) fn mark_island_readers(&mut self) {
        let readers: Vec<usize> = (0..self.layers.len())
            .filter(|&i| {
                let l = &self.layers[i];
                l.filters
                    .iter()
                    .chain(l.mask.iter().flat_map(|m| m.filters.iter()))
                    .any(|e| e.settings.reads_islands())
            })
            .collect();
        for &i in &readers {
            self.mark_layer(i, None);
        }
        if !readers.is_empty() {
            self.mark_clipped_layers();
        }
    }

    /// 今の入力。
    pub fn effect_inputs(&self) -> &EffectInputs {
        &self.effects.inputs
    }

    /// 入力を読むレイヤー（Generator の段のあるレイヤー・画像やグラデーションやデカールの塗りつぶし）が出す所に印を付ける。
    fn mark_input_readers(&mut self) {
        let mut any = false;
        for i in 0..self.layers.len() {
            let l = &self.layers[i];
            let reads = l.reads_inputs_for_fill()
                || l.filters.iter().any(|e| e.settings.is_generator())
                || l.mask
                    .as_ref()
                    .is_some_and(|m| m.filters.iter().any(|e| e.settings.is_generator()));
            if reads {
                any = true;
                self.mark_layer(i, None);
            }
        }
        if any {
            self.mark_clipped_layers();
        }
    }

    // ───────── 作業メモリ・キャッシュの予算 ─────────

    /// 1 ブロックの評価が使える作業メモリ（既定 256 MiB）。今のスタックが要る量より小さくはできない。
    pub fn filter_working_budget_bytes(&self) -> u64 {
        self.effects.working_budget
    }
    pub fn set_filter_working_budget_bytes(&mut self, bytes: u64) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let need = self.current_working_need(self.effects.block_pixels)?;
        if bytes < need {
            return Err(CoreError::InvalidArgument(
                "予算が今のフィルターの要る量より小さい",
            ));
        }
        if bytes != self.effects.working_budget {
            self.effects.working_budget = bytes;
            // 継ぎ目をまたげるか（使うとブロックの作業メモリが予算を超えるか）が変わり得る: またぐレイヤーを描き直す
            self.mark_seam_readers();
            self.release_effect_cache();
        }
        Ok(())
    }
    /// 評価のブロックの一辺（画素。1〜4096。タイルの大きさの倍数に切り下げ、最低 1 タイル）。結果はブロックの大きさによらない。
    pub fn filter_block_pixels(&self) -> u32 {
        self.effects.block_pixels
    }
    pub fn set_filter_block_pixels(&mut self, pixels: u32) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if pixels == 0 || pixels > 4096 {
            return Err(CoreError::InvalidArgument("block_pixels"));
        }
        if self.current_working_need(pixels)? > self.effects.working_budget {
            return Err(CoreError::WorkingBudgetExceeded);
        }
        if pixels != self.effects.block_pixels {
            // 評価済みのブロックの鍵（レイヤー・元・ブロックの番号）はブロックの大きさを含まない: 大きさを替えたら、前の大きさのブロックを返さない
            self.effects.block_pixels = pixels;
            self.release_effect_cache();
        }
        Ok(())
    }
    fn current_working_need(&self, block_pixels: u32) -> Result<u64, CoreError> {
        let mut need = 0u64;
        for l in &self.layers {
            for c in Channel::ALL {
                let chain = l.active_chain(c);
                if !chain.is_empty() {
                    need = need.max(self.block_need_with(&chain, block_pixels)?);
                }
            }
            if let Some(m) = &l.mask {
                let chain: Vec<&FilterEffect> =
                    m.filters.iter().filter(|e| e.is_active()).collect();
                if !chain.is_empty() {
                    need = need.max(self.block_need_with(&chain, block_pixels)?);
                }
            }
        }
        Ok(need)
    }
    /// 評価済みのタイルを持っておく予算（派生の表示用。古いものから捨てる。0 なら持たない）。
    pub fn filter_cache_budget_bytes(&self) -> u64 {
        self.effects.cache_budget
    }
    pub fn set_filter_cache_budget_bytes(&mut self, bytes: u64) {
        self.effects.cache_budget = bytes;
        self.trim_effect_cache();
    }
    /// 画像のミップマップを持っておく予算（既定 256 MiB）。
    pub fn fill_image_cache_budget_bytes(&self) -> u64 {
        self.effects.image_cache_budget
    }
    pub fn set_fill_image_cache_budget_bytes(&mut self, bytes: u64) {
        if bytes == self.effects.image_cache_budget {
            return;
        }
        self.effects.image_cache_budget = bytes;
        // 予算で使えなかった（使えるようになった）画像があり得るので、画像のレイヤーを描き直す
        self.effects.inputs_revision += 1;
        self.mark_input_readers();
        self.trim_effect_cache();
    }

    // ───────── 状態・効かない効果の一覧 ─────────

    /// Generator の段が今は入力のまま通しているなら、その理由。使えるなら None。
    pub fn generator_inactive(
        &self,
        layer: LayerId,
        filter: FilterId,
    ) -> Result<Option<InactiveReason>, CoreError> {
        let index = self.index_of(layer)?;
        let (_, target, at) = self.locate_filter(layer, filter)?;
        let EffectSettings::Generator(g) = &self.stack_ref(index, target)?[at].settings else {
            return Err(CoreError::Unsupported("ジェネレーターではない"));
        };
        Ok(self.generator_reason(g, index))
    }

    /// ノイズ・グランジの段が、位置のマップが使えなくて UV の空間で評価しているなら、その理由（値は出ている。入力のまま通す
    /// [`Self::generator_inactive`] とは別）。位置で評価できる・ほかの種類なら None。
    pub fn generator_fallback(
        &self,
        layer: LayerId,
        filter: FilterId,
    ) -> Result<Option<generator::Inactive>, CoreError> {
        let index = self.index_of(layer)?;
        let (_, target, at) = self.locate_filter(layer, filter)?;
        let EffectSettings::Generator(g) = &self.stack_ref(index, target)?[at].settings else {
            return Err(CoreError::Unsupported("ジェネレーターではない"));
        };
        Ok(self.generator_status(g, index).1)
    }

    /// 塗りつぶしのチャンネルのグラデーションが今は値を見せているなら、その理由。
    pub fn fill_gradient_inactive(
        &self,
        layer: LayerId,
        channel: Channel,
    ) -> Result<Option<InactiveReason>, CoreError> {
        let index = self.fill_layer_index(layer)?;
        let g = self.layers[index]
            .fill_gradients
            .get(&channel)
            .ok_or(CoreError::Unsupported(
                "そのチャンネルにグラデーションが無い",
            ))?;
        Ok(self.generator_reason(g, index))
    }

    /// 効かない効果の一覧（保存・書き出しの知らせに使う: 合成に効果が入っていないことを黙っていない）。1 行に 1 つ。
    /// 画面の言語で出すときは [`Self::inactive_effect_list`] を使う。
    pub fn inactive_effects(&self) -> Vec<String> {
        self.inactive_effect_list()
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// 位置のマップが使えなくて UV の空間で評価している、有効なノイズ・グランジの段の一覧（効いてはいる。位置の継ぎ目の無さと回転が効かない）。
    pub fn fallback_effect_list(&self) -> Vec<FallbackEffect> {
        let mut out = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let stacks = [
                (false, l.filters.as_slice()),
                (
                    true,
                    l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
                ),
            ];
            for (mask, stack) in stacks {
                for e in stack {
                    if let (EffectSettings::Generator(g), true) = (&e.settings, e.is_active()) {
                        if !g.kind.is_procedural() {
                            continue;
                        }
                        if let Some(why) = self.generator_status(g, i).1 {
                            out.push(FallbackEffect {
                                layer: l.id,
                                layer_name: l.name.clone(),
                                mask,
                                kind: g.kind,
                                reason: InactiveReason::Generator(why),
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// 効かない効果（有効な Generator の段で使えるマップ・Anchor が無いもの・値を見せているグラデーションと画像・出ていないデカール）の一覧。
    pub fn inactive_effect_list(&self) -> Vec<InactiveEffect> {
        let mut out = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let stacks = [
                (false, l.filters.as_slice()),
                (
                    true,
                    l.mask.as_ref().map_or(&[][..], |m| m.filters.as_slice()),
                ),
            ];
            let mut push = |target, reason| {
                out.push(InactiveEffect {
                    layer: l.id,
                    layer_name: l.name.clone(),
                    target,
                    reason,
                });
            };
            for (mask, stack) in stacks {
                for e in stack {
                    if let (EffectSettings::Generator(g), true) = (&e.settings, e.is_active()) {
                        if let Some(why) = self.generator_reason(g, i) {
                            push(InactiveTarget::Generator { mask, kind: g.kind }, why);
                        }
                    }
                }
            }
            for (c, g) in &l.fill_gradients {
                if l.is_channel_enabled(*c) {
                    if let Some(why) = self.generator_reason(g, i) {
                        push(InactiveTarget::FillGradient(*c), why);
                    }
                }
            }
            if l.is_decal() && l.fill.keys().any(|c| l.is_channel_enabled(*c)) {
                if let Some(why) = self.decal_problem(i) {
                    push(InactiveTarget::Decal, why);
                }
            }
            for (c, id) in &l.fill_images {
                if !l.is_channel_enabled(*c) {
                    continue;
                }
                if let Some(why) = self.fill_image_problem(i, *c, *id) {
                    push(InactiveTarget::FillImage(*c), why);
                }
            }
            for (c, g) in &l.fill_points {
                if l.is_channel_enabled(*c) {
                    if let Some(why) = self.points_reason(g) {
                        push(InactiveTarget::FillPoints(*c), why);
                    }
                }
            }
        }
        out
    }
}

/// 塗りつぶしの点のグラデーションの検査。
fn validate_fill_points(channel: Channel, g: &PointGradient) -> Result<(), CoreError> {
    if channel == Channel::Normal {
        return Err(CoreError::Unsupported(
            "点のグラデーションは色かスカラーで、法線ではない",
        ));
    }
    g.validate().map_err(CoreError::InvalidArgument)
}

/// 塗りつぶしのグラデーションの設定の検査（C# の `ValidateFillGradient`）。
fn validate_fill_gradient(channel: Channel, g: &generator::Settings) -> Result<(), CoreError> {
    if channel == Channel::Normal {
        return Err(CoreError::Unsupported(
            "形のグラデーションは色かスカラーで、法線ではない",
        ));
    }
    if g.kind != generator::Kind::ShapeGradient || g.ramp.is_none() {
        return Err(CoreError::InvalidArgument(
            "塗りつぶしのグラデーションはランプ付きの形のグラデーション",
        ));
    }
    if g.blend != generator::Blend::Replace {
        return Err(CoreError::InvalidArgument(
            "塗りつぶしのグラデーションは置き換え",
        ));
    }
    g.validate().map_err(|e| match e {
        generator::Error::Invalid(why) => CoreError::InvalidArgument(why),
        _ => CoreError::InvalidArgument("グラデーションの設定"),
    })
}
