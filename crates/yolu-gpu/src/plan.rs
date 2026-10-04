//! 文書の層を、GPU の合成が読む平らな並び（層ごとの設定と、GPU に上げる面の番号）に直す。
//! `yolu-core` の CPU の合成（`composite.rs` の計画）と同じ規則で、描く層・落とす層・クリッピングの組を決める。
//! GPU の合成が扱えない文書（調整の層・独立して合成するグループ・法線の種類のチャンネル）は [`Unsupported`] で断る。
//! 断ったあとの CPU の合成は呼び手の仕事で、ここは何も変えない。
use super::LayerData;
use std::{collections::HashMap, fmt};
use yolu_core::{BlendMode, Channel, ChannelKind, Document, Layer, LayerId, LayerKind, Rgba8};

/// 値のない番号（面を持たない・マスクがない）。シェーダーの `NONE` と同じ。
pub(crate) const NONE: u32 = u32::MAX;

/// GPU の合成が扱えない理由。扱えない文書は CPU で合成する。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unsupported {
    /// 文書にないチャンネル。
    UnknownChannel,
    /// 法線の種類のチャンネル（単位ベクトルの合成式）。
    NormalChannel,
    /// 描く調整の層（下の合成を読んで変える）。
    AdjustmentLayer,
    /// 中身を透明から合成してから重ねるグループ（通過でない・不透明度が 1 でない・マスクが効く・クリッピングされる・クリッピングの下地）。
    IsolatedGroup,
}

impl Unsupported {
    /// 理由の文（日本語。画面に出す文言は呼び手が種類から作る）。
    pub fn reason(self) -> &'static str {
        match self {
            Unsupported::UnknownChannel => "文書にないチャンネル",
            Unsupported::NormalChannel => "法線の種類のチャンネルは GPU で合成できない",
            Unsupported::AdjustmentLayer => "調整の層は GPU で合成できない",
            Unsupported::IsolatedGroup => "独立して合成するグループは GPU で合成できない",
        }
    }
}

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason())
    }
}
impl std::error::Error for Unsupported {}

/// この文書・チャンネルを GPU で合成できるか。層の並びだけを見る軽い確認（画素には触れない）。
pub fn supports(doc: &Document, channel: Channel) -> Result<(), Unsupported> {
    Plan::build(doc, channel).map(|_| ())
}

/// 上げる面の元。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slot {
    /// `doc.layers()` の添字。
    pub layer: usize,
    /// true ならマスクの面（アルファが隠す量）、false ならチャンネルの面。
    pub mask: bool,
}

/// GPU の合成の計画: 合成の段（下から上の順）と、上げる面の並び。
pub(crate) struct Plan {
    pub entries: Vec<LayerData>,
    pub slots: Vec<Slot>,
}

impl Plan {
    pub(crate) fn metadata(&self) -> Vec<LayerData> {
        self.entries.clone()
    }

    pub(crate) fn build(doc: &Document, channel: Channel) -> Result<Plan, Unsupported> {
        let kind = doc
            .channel_info(channel)
            .ok_or(Unsupported::UnknownChannel)?
            .kind;
        if kind == ChannelKind::Normal {
            return Err(Unsupported::NormalChannel);
        }
        let layers = doc.layers();
        let mut children: HashMap<Option<LayerId>, Vec<usize>> = HashMap::new();
        for (i, l) in layers.iter().enumerate() {
            children.entry(l.parent()).or_default().push(i);
        }
        let builder = Builder {
            layers,
            children,
            channel,
            kind,
        };
        let mut list = Vec::new();
        builder.level(None, &mut list)?;
        let mut slots = Vec::new();
        let mut entries = Vec::with_capacity(list.len());
        for (index, clip) in list {
            let l = &layers[index];
            let mut data = LayerData {
                opacity: l.opacity_in(channel) as f32,
                mode: l.blend_mode_in(channel) as u32,
                clip: u32::from(clip),
                slot: NONE,
                mask: NONE,
                fill: 0,
                mask_invert: 0,
                mask_density: 0.0,
            };
            if l.kind() == LayerKind::Fill {
                let c = l.fill_value(channel).unwrap_or(Rgba8::TRANSPARENT);
                data.fill = u32::from_le_bytes(c.to_array());
            } else {
                data.slot = slots.len() as u32;
                slots.push(Slot {
                    layer: index,
                    mask: false,
                });
            }
            if let Some(m) = l.mask().filter(|m| !m.is_neutral()) {
                data.mask = slots.len() as u32;
                data.mask_invert = u32::from(m.inverted());
                data.mask_density = m.density() as f32;
                slots.push(Slot {
                    layer: index,
                    mask: true,
                });
            }
            entries.push(data);
        }
        Ok(Plan { entries, slots })
    }
}

struct Builder<'a> {
    layers: &'a [Layer],
    children: HashMap<Option<LayerId>, Vec<usize>>,
    channel: Channel,
    kind: ChannelKind,
}

impl Builder<'_> {
    /// その層がこのチャンネルで何かを出せるか（CPU の `Stack::active`）。グループは対象外。
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

    /// 1 つの段（親の子）を平らな並びへ。`(層の添字, クリッピングの印)` を下から上へ足す。
    fn level(
        &self,
        parent: Option<LayerId>,
        out: &mut Vec<(usize, bool)>,
    ) -> Result<(), Unsupported> {
        let Some(siblings) = self.children.get(&parent) else {
            return Ok(());
        };
        let mut k = 0;
        while k < siblings.len() {
            // 一番下の兄弟は印があっても下地。そのあとに続く印のある層が、この下地の組に入る。
            let mut end = k + 1;
            while end < siblings.len() && self.layers[siblings[end]].clipping() {
                end += 1;
            }
            let base = siblings[k];
            let clips = &siblings[k + 1..end];
            k = end;
            let l = &self.layers[base];
            match l.kind() {
                LayerKind::Group => {
                    let mut inner = Vec::new();
                    if !self.group_drawn(l, &mut inner)? {
                        continue;
                    }
                    // 通過でなければ、中身を透明から合成して重ねる（GPU では扱わない）。
                    // 不透明度 1・マスクが効かない通過は、中身をそのまま下へ重ねるのと同じ。クリッピングの組に入るのは
                    // 描かれる層だけ（core の `plan_level` は `make_entry` が None の層を `clips` に入れない）なので、
                    // 見えない・不透明度 0 の層が上に並んでいるだけでは、組を持たない通過のままにする。
                    let mut clipped = false;
                    for &c in clips {
                        if self.clip_drawn(&self.layers[c])? {
                            clipped = true;
                            break;
                        }
                    }
                    let transparent = !clipped
                        && l.blend_mode_in(self.channel) == BlendMode::PassThrough
                        && l.opacity_in(self.channel) == 1.0
                        && l.mask().is_none_or(|m| m.is_neutral());
                    if !transparent {
                        return Err(Unsupported::IsolatedGroup);
                    }
                    out.extend(inner);
                }
                LayerKind::Adjustment => {
                    if self.active(l) {
                        return Err(Unsupported::AdjustmentLayer);
                    }
                }
                LayerKind::Raster | LayerKind::Fill => {
                    if !self.active(l) {
                        continue; // 下地が落ちれば、クリッピングの層も一緒に落ちる
                    }
                    out.push((base, false));
                    for &c in clips {
                        let cl = &self.layers[c];
                        match cl.kind() {
                            LayerKind::Group => {
                                if self.group_drawn(cl, &mut Vec::new())? {
                                    return Err(Unsupported::IsolatedGroup);
                                }
                            }
                            LayerKind::Adjustment => {
                                if self.active(cl) {
                                    return Err(Unsupported::AdjustmentLayer);
                                }
                            }
                            LayerKind::Raster | LayerKind::Fill => {
                                if self.active(cl) {
                                    out.push((c, true));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// 下地のクリッピングの組に入る層か（CPU の `make_entry` が Some。見えない・不透明度 0・チャンネルが無効・中身が無い層は入らない）。
    fn clip_drawn(&self, c: &Layer) -> Result<bool, Unsupported> {
        match c.kind() {
            LayerKind::Group => self.group_drawn(c, &mut Vec::new()),
            LayerKind::Adjustment | LayerKind::Raster | LayerKind::Fill => Ok(self.active(c)),
        }
    }

    /// グループが何かを描くか（CPU の `make_entry`: 見えない・不透明度 0・中身が無いなら落ちる）。描くなら中身を `inner` に平らにして返す。
    fn group_drawn(&self, g: &Layer, inner: &mut Vec<(usize, bool)>) -> Result<bool, Unsupported> {
        if !g.visible() || g.opacity_in(self.channel) <= 0.0 {
            return Ok(false);
        }
        self.level(Some(g.id()), inner)?;
        Ok(!inner.is_empty())
    }
}
