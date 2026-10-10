use super::{Command, Document};
use crate::smart::{
    check_name, SmartKind, SmartMaterial, SmartPlaceResult, SmartPlacement, SmartResampling,
};
use crate::{Channel, ChannelKind, CoreError, Layer, LayerId, LayerKind};
use std::collections::{BTreeSet, HashMap, HashSet};

/// レイヤーのパスの一覧を外す。ラスターレイヤーは画素をそのまま残して true を返す。塗りつぶしレイヤーのパスの画素はレイヤーの面にしか無く、一覧が
/// 無いと評価で使われない（保存も、画素を持てないレイヤーとして断られる）ので、パスのチャンネルの面も外し（値の無いチャンネルは
/// パスを付けるときに有効にしたものなので無効に戻し）、false を返す。
fn detach_paths(layer: &mut Layer) -> bool {
    let entries = std::mem::take(&mut layer.paths);
    if layer.kind != LayerKind::Fill {
        return true;
    }
    for c in crate::paths::list_channels(&entries) {
        layer.put_surface(c, None);
        if !layer.fill.contains_key(&c) {
            layer.set_enabled(c, false);
        }
    }
    false
}

impl Document {
    /// 選択したグループの子孫を含め、文書の並び順で独立した写しを作る。
    pub fn capture_smart_material(
        &self,
        ids: &[LayerId],
        name: &str,
    ) -> Result<SmartMaterial, CoreError> {
        self.ensure_no_stroke()?;
        check_name(name)?;
        if ids.is_empty() {
            return Err(CoreError::InvalidArgument("保存するレイヤーがありません"));
        }
        for &id in ids {
            self.index_of(id)?;
        }
        let selected: HashSet<_> = ids.iter().copied().collect();
        let mut layers: Vec<_> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                selected.contains(&l.id) || selected.iter().any(|id| self.is_descendant(*i, *id))
            })
            .map(|(_, l)| l.clone())
            .collect();
        let held: HashSet<_> = layers.iter().map(|l| l.id).collect();
        let mut notes = Vec::new();
        for l in &mut layers {
            if l.parent.is_some_and(|p| !held.contains(&p)) {
                l.parent = None;
            }
            // モデルの上のパスは、そのモデルの三角形に結び付いている: ラスターレイヤーは画素だけが残る（C# の SmartMaterials と同じ）。
            // 塗りつぶしレイヤーは画素を持たない（パスの画素は一覧が無いと評価で使われない）ので、パスごと外れる
            if matches!(l.path(), Some(crate::LayerPath::Surface(_))) {
                let kept = detach_paths(l);
                notes.push(if kept {
                    format!(
                        "「{}」のモデルの上のパスは画素だけになりました（パスはそのモデルの三角形に結び付く）",
                        l.name
                    )
                } else {
                    format!(
                        "「{}」のモデルの上のパスは外れました（パスはそのモデルの三角形に結び付き、塗りつぶしレイヤーは画素を持たない）",
                        l.name
                    )
                });
            }
            // 定規は文書のレイヤーに付く物で、.ylsmart（版 21 まで）にも入らない: 外れる
            if !l.rulers.is_empty() {
                notes.push(format!(
                    "「{}」の定規は外れました（スマートマテリアルは定規を持たない）",
                    l.name
                ));
                l.rulers.clear();
            }
            // テキストの値はフォント（文書の外のファイル）に結び付き、.ylsmart（版 21 まで）にも入らない: 画素だけが残る
            if l.text.take().is_some() {
                notes.push(format!(
                    "「{}」のテキストは画素だけになりました（スマートマテリアルはテキストの値を持たない）",
                    l.name
                ));
            }
        }
        let mut material = self
            .smart_fragment(SmartKind::Material, name, layers)
            .with_fresh_ids();
        material.notes = notes;
        Ok(material)
    }
    /// 塗りつぶしレイヤー 1 つを、マテリアル（置くと新しい塗りつぶしレイヤーになる素材）として写す。持つのは塗りつぶしの見た目を決める物
    /// （チャンネルの値と有効の印・画像と投影・グラデーション・レイヤーの画素のフィルター）で、レイヤーの並びの中の置き場に結び付く物（マスク・
    /// Anchor・クリッピング・不透明度・合成モード・チャンネルごとの合成・隠した状態・ロック）は持たず、新しい塗りつぶしレイヤーの既定にする。
    /// 塗りつぶしレイヤーでなければ断る。
    pub fn capture_material(&self, id: LayerId, name: &str) -> Result<SmartMaterial, CoreError> {
        if self.layers[self.index_of(id)?].kind != LayerKind::Fill {
            return Err(CoreError::InvalidArgument(
                "塗りつぶしレイヤーではありません",
            ));
        }
        let mut material = self.capture_smart_material(&[id], name)?;
        for l in &mut material.layers {
            l.mask = None;
            l.anchor = None;
            l.clipping = false;
            l.opacity = 1.0;
            l.blend_mode = crate::BlendMode::Normal;
            l.blends.clear();
            l.visible = true;
            l.locks = crate::LayerLocks::NONE;
        }
        Ok(material)
    }
    pub fn capture_smart_mask(&self, id: LayerId, name: &str) -> Result<SmartMaterial, CoreError> {
        self.ensure_no_stroke()?;
        check_name(name)?;
        let mask = self.layers[self.index_of(id)?]
            .mask
            .clone()
            .ok_or(CoreError::InvalidArgument("保存するマスクがありません"))?;
        let mut holder = Layer::new(id, name, LayerKind::Fill);
        holder.mask = Some(mask);
        Ok(self
            .smart_fragment(SmartKind::Mask, name, vec![holder])
            .with_fresh_ids())
    }
    /// 断片が持つチャンネル定義は、標準の 6 つと、レイヤーが何かを持つユーザーチャンネルだけ（無効でも面・値・合成を持つものは含む）。
    /// 使っていない定義を持ち込むと、保存の版がユーザーチャンネルの版に上がって Unity 版が読めなくなる。
    fn smart_fragment(&self, kind: SmartKind, name: &str, layers: Vec<Layer>) -> SmartMaterial {
        let mut channel_info: Vec<_> = self
            .channels
            .iter()
            .enumerate()
            .map(|(i, info)| {
                info.clone().filter(|_| {
                    i < Channel::STANDARD_COUNT
                        || Channel::from_index(i)
                            .is_some_and(|c| layers.iter().any(|l| l.has_channel_data(c)))
                })
            })
            .collect();
        while channel_info.len() > Channel::STANDARD_COUNT
            && channel_info.last().is_some_and(Option::is_none)
        {
            channel_info.pop();
        }
        SmartMaterial {
            document_id: self.id,
            normal_settings: self.normal_settings,
            kind,
            name: name.into(),
            width: self.width,
            height: self.height,
            tile_size: self.tile_size,
            layers,
            channel_info,
            notes: Vec::new(),
        }
    }
    /// 保存された断片を検証して取り込む。渡した文書の履歴は保持しない。
    pub fn into_smart_material(
        self,
        kind: SmartKind,
        name: &str,
    ) -> Result<SmartMaterial, CoreError> {
        self.ensure_no_stroke()?;
        check_name(name)?;
        self.validate_structure()?;
        if self.layers.is_empty() {
            return Err(CoreError::InvalidArgument("空のスマート素材"));
        }
        if kind == SmartKind::Mask
            && (self.layers.len() != 1
                || self.layers[0].kind != LayerKind::Fill
                || !self.layers[0].fill.is_empty()
                || self.layers[0].mask.is_none())
        {
            return Err(CoreError::InvalidArgument("スマートマスクの断片が不正"));
        }
        Ok(self.smart_fragment(kind, name, self.layers.clone()))
    }
    pub fn place_smart_material(
        &mut self,
        material: &SmartMaterial,
        placement: &SmartPlacement,
    ) -> Result<SmartPlaceResult, CoreError> {
        self.ensure_no_stroke()?;
        if material.kind != SmartKind::Material {
            return Err(CoreError::InvalidArgument(
                "スマートマスクはレイヤーに置けません",
            ));
        }
        if let Some(p) = placement.parent {
            if !self.layers[self.index_of(p)?].is_group() {
                return Err(CoreError::InvalidArgument("配置先がグループではありません"));
            }
        }
        self.check_user_channels(material)?;
        let wrap = material
            .layers
            .iter()
            .filter(|l| l.parent.is_none())
            .count()
            > 1;
        if self.layers.len() + material.layers.len() + usize::from(wrap) > 2048 {
            return Err(CoreError::InvalidArgument("レイヤーは2048個までです"));
        }
        self.ensure_nesting_room(
            placement.parent,
            super::structure::group_chain_height(&material.layers) + usize::from(wrap),
        )?;
        let siblings: Vec<_> = self
            .layers
            .iter()
            .enumerate()
            .filter(|(_, l)| l.parent == placement.parent)
            .map(|(i, _)| i)
            .collect();
        let index = match placement.position.and_then(|p| siblings.get(p)).copied() {
            Some(mut i) => {
                let id = self.layers[i].id;
                while i > 0 && self.is_descendant(i - 1, id) {
                    i -= 1;
                }
                i
            }
            None => placement
                .parent
                .map(|p| self.index_of(p))
                .transpose()?
                .unwrap_or(self.layers.len()),
        };
        let (mut copies, notes) = self.smart_layers_at_size(
            material,
            placement.resampling,
            self.source_budget.saturating_sub(self.allocated_bytes()),
        )?;
        let ids: HashMap<_, _> = copies.iter().map(|l| (l.id, self.new_layer_id())).collect();
        let group = wrap.then(|| self.new_layer_id());
        // 配置のたびに、段・Anchor は新しい ID（写しの中の Anchor を読む段は写しの Anchor を読む）。置き先の決まりに収まらなければ断る
        self.renew_effect_ids(&mut copies);
        self.check_layer_effects(&copies)?;
        let mut off = BTreeSet::new();
        for l in &mut copies {
            l.id = ids[&l.id];
            l.parent = l.parent.map(|p| ids[&p]).or(group).or(placement.parent);
            if let Some(keep) = &placement.channels {
                for c in material.channels() {
                    if l.is_channel_enabled(c) && !keep.contains(&c) {
                        l.set_enabled(c, false);
                        off.insert(c);
                    }
                }
            }
        }
        if let Some(id) = group {
            let mut l = Layer::new(
                id,
                placement.group_name.as_deref().unwrap_or(&material.name),
                LayerKind::Group,
            );
            l.parent = placement.parent;
            copies.push(l);
        }
        let layer_id = copies
            .iter()
            .rev()
            .find(|l| l.parent == placement.parent)
            .expect("最上位のレイヤー")
            .id;
        let result = SmartPlaceResult {
            layer_id,
            layers: copies.iter().map(|l| l.id).collect(),
            switched_off: off.into_iter().collect(),
            resampled: material.width != self.width || material.height != self.height,
            replaced_mask: false,
            notes,
        };
        let cost = 128 + copies.iter().map(Layer::allocated_bytes).sum::<u64>();
        self.execute(
            Command::Insert {
                index,
                len: copies.len(),
                block: Some(copies),
            },
            cost,
        )?;
        Ok(result)
    }
    /// ユーザーチャンネルは番号だけで別の意味へ結び付けない。レイヤーが何かを持つ（有効・面・塗りつぶしの値・チャンネルごとの合成。
    /// 無効にしても面・値・合成は残る）番号は、配置先に同じ番号で同じ情報のチャンネルが無ければ断る。落として配置はしない。
    fn check_user_channels(&self, material: &SmartMaterial) -> Result<(), CoreError> {
        for i in Channel::STANDARD_COUNT..Channel::MAX {
            let channel = Channel::from_index(i).expect("チャンネル番号");
            if !material.layers.iter().any(|l| l.has_channel_data(channel)) {
                continue;
            }
            let info = material.channel_info.get(i).and_then(Option::as_ref);
            if info.is_none() || self.channel_info(channel) != info {
                return Err(CoreError::Unsupported(
                    "ユーザーチャンネルの対応が一致しません",
                ));
            }
        }
        Ok(())
    }
    pub fn apply_smart_mask(
        &mut self,
        material: &SmartMaterial,
        id: LayerId,
        resampling: Option<SmartResampling>,
    ) -> Result<SmartPlaceResult, CoreError> {
        self.ensure_no_stroke()?;
        if material.kind != SmartKind::Mask {
            return Err(CoreError::InvalidArgument(
                "スマートマテリアルはマスクに置けません",
            ));
        }
        let i = self.index_of(id)?;
        // マスクを変えるのは、すべてのロックでだけ断る（add_layer_mask と同じ。C# の ApplySmartMask の RefuseLockedAttributes）
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        let old = self.layers[i]
            .mask
            .as_ref()
            .map_or(0, |m| m.surface.allocated_bytes());
        let (mut copies, notes) = self.smart_layers_at_size(
            material,
            resampling,
            self.source_budget
                .saturating_sub(self.allocated_bytes() - old),
        )?;
        // 置くたびに、段・Anchor は新しい ID（同じスマートマスクを何度置いても、元のレイヤーへ戻しても、文書の中で ID が重ならない）
        self.renew_effect_ids(&mut copies);
        self.check_layer_effects(&copies)?;
        let mask = copies[0].mask.take().expect("検証済みのマスク");
        let result = SmartPlaceResult {
            layer_id: id,
            layers: vec![id],
            switched_off: vec![],
            resampled: material.width != self.width || material.height != self.height,
            replaced_mask: self.layers[i].mask.is_some(),
            notes,
        };
        let cost = 64 + mask.surface.allocated_bytes();
        self.execute(
            Command::SwapSmartMask {
                id,
                mask: Some(Box::new(mask)),
            },
            cost,
        )?;
        Ok(result)
    }
    fn smart_layers_at_size(
        &self,
        material: &SmartMaterial,
        how: Option<SmartResampling>,
        room: u64,
    ) -> Result<(Vec<Layer>, Vec<String>), CoreError> {
        let mut layers = material.layers.clone();
        let mut notes = Vec::new();
        let same = material.width == self.width && material.height == self.height;
        if !same {
            // 画素で測るもの（ぼかし・シャープの半径）は倍率に合わせる。縦横の倍率が違うときは幾何平均
            let scale = (f64::from(self.width) / f64::from(material.width)
                * (f64::from(self.height) / f64::from(material.height)))
            .sqrt();
            notes = Document::scale_effect_radii(&mut layers, scale);
            // 大きさの違うキャンバスでは、キャンバスの上のパスの点は元の大きさのまま: ラスターレイヤーは画素だけが残り、塗りつぶしレイヤーはパスごと外れる
            for l in &mut layers {
                if l.has_paths() {
                    notes.push(if detach_paths(l) {
                        format!(
                            "「{}」のパスは画素だけになりました（点は元のキャンバスの大きさのもの）",
                            l.name
                        )
                    } else {
                        format!(
                            "「{}」のパスは外れました（点は元のキャンバスの大きさのもので、塗りつぶしレイヤーは画素を持たない）",
                            l.name
                        )
                    });
                }
            }
        }
        if same && material.tile_size == self.tile_size {
            if material.pixel_bytes() > room {
                return Err(CoreError::SourceBudgetExceeded);
            }
            return Ok((layers, notes));
        }
        let how = if same {
            SmartResampling::Nearest
        } else {
            how.unwrap_or(
                if self.width >= material.width && self.height >= material.height {
                    SmartResampling::Bilinear
                } else {
                    SmartResampling::Area
                },
            )
        };
        let mut remaining = room;
        for layer in &mut layers {
            for (i, s) in layer.surfaces.iter_mut().enumerate() {
                if let Some(surface) = s {
                    let normal = material
                        .channel_info
                        .get(i)
                        .and_then(Option::as_ref)
                        .is_some_and(|c| c.kind == ChannelKind::Normal);
                    *surface = super::smart_resample::surface(
                        surface,
                        self.width,
                        self.height,
                        self.tile_size,
                        how,
                        normal,
                        remaining,
                    )?;
                    remaining = remaining
                        .checked_sub(surface.allocated_bytes())
                        .ok_or(CoreError::SourceBudgetExceeded)?;
                }
            }
            if let Some(mask) = &mut layer.mask {
                mask.surface = super::smart_resample::surface(
                    &mask.surface,
                    self.width,
                    self.height,
                    self.tile_size,
                    how,
                    false,
                    remaining,
                )?;
                remaining = remaining
                    .checked_sub(mask.surface.allocated_bytes())
                    .ok_or(CoreError::SourceBudgetExceeded)?;
            }
        }
        Ok((layers, notes))
    }
}
impl SmartMaterial {
    fn with_fresh_ids(mut self) -> Self {
        self.document_id = super::random_id(0);
        self.normal_settings = crate::NormalSettings::DEFAULT;
        let ids: HashMap<_, _> = self
            .layers
            .iter()
            .enumerate()
            .map(|(i, l)| (l.id, LayerId(super::random_id(i as u64 + 1))))
            .collect();
        for l in &mut self.layers {
            l.id = ids[&l.id];
            l.parent = l.parent.map(|p| ids[&p]);
        }
        self
    }
    /// 保存用の独立した文書。画素は書き込み時に分離する。
    pub fn fragment_document(&self) -> Result<Document, CoreError> {
        let mut d = Document::with_tile_size(self.width, self.height, self.tile_size)?;
        d.id = self.document_id;
        d.normal_settings = self.normal_settings;
        d.layers = self.layers.clone();
        d.channels = self.channel_info.clone();
        Ok(d)
    }
}
