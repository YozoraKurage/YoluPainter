//! 層の中身の設定の編集（C# の AddLayerMask・RemoveLayerMask・SetLayerMask*・SetFillValue・SetAdjustment・SetChannelEnabled・
//! SetChannelBlend・SetNormalSettings）。どれも 1 回の Undo で、断ったら何も変えない。スライダーのドラッグはまとめられる。

use super::{CoalesceKey, Command, Document, Property};
use crate::adjust::AdjustmentSettings;
use crate::error::CoreError;
use crate::layer::{ChannelBlend, LayerId, RasterMask};
use crate::math::require_finite;
use crate::normal::NormalSettings;
use crate::surface::Surface;
use crate::types::{BlendMode, Channel, LayerKind, Rgba8};

impl Document {
    // ───────── マスク ─────────

    /// 空のラスターマスク（何も隠さない）を層に足す。どの種類の層にも 1 つまで。1 回の Undo。
    pub fn add_layer_mask(&mut self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        // マスクの有無がロックより先（C# の AddLayerMask）
        if self.layers[index].mask.is_some() {
            return Err(CoreError::Unsupported("層はもうマスクを持っている"));
        }
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        let mask = RasterMask::new(Surface::new(self.width, self.height, self.tile_size));
        self.execute(
            Command::AddMask {
                id,
                mask: Some(Box::new(mask)),
            },
            64,
        )
    }

    /// 層のマスクを外す。1 回の Undo で同じマスク（画素・設定）が戻る。
    pub fn remove_layer_mask(&mut self, id: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(id)?;
        // マスクの有無がロックより先（C# の RemoveLayerMask）
        let bytes = Self::mask_of(&self.layers[index])?
            .surface
            .allocated_bytes();
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        self.execute(Command::RemoveMask { id, mask: None }, 64 + bytes)
    }

    pub fn set_layer_mask_enabled(&mut self, id: LayerId, enabled: bool) -> Result<(), CoreError> {
        self.set_property(id, Property::MaskEnabled(enabled), None)
    }

    pub fn set_layer_mask_inverted(
        &mut self,
        id: LayerId,
        inverted: bool,
    ) -> Result<(), CoreError> {
        self.set_property(id, Property::MaskInverted(inverted), None)
    }

    /// マスクの濃度 0〜1。coalesce ならドラッグの続きをまとめる。
    pub fn set_layer_mask_density(
        &mut self,
        id: LayerId,
        density: f64,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        require_finite(density, "density")?;
        if !(0.0..=1.0).contains(&density) {
            return Err(CoreError::InvalidArgument("density"));
        }
        self.set_property(
            id,
            Property::MaskDensity(density),
            coalesce.then_some(CoalesceKey::MaskDensity(id)),
        )
    }

    // ───────── 塗りつぶし・調整 ─────────

    /// 塗りつぶしの層のチャンネルの値を置く（None で消す）。値を置くとそのチャンネルを有効にする。coalesce ならドラッグをまとめる。
    pub fn set_fill_value(
        &mut self,
        id: LayerId,
        channel: Channel,
        value: Option<Rgba8>,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let index = self.index_of(id)?;
        let layer = &self.layers[index];
        if layer.kind != LayerKind::Fill {
            return Err(CoreError::Unsupported("塗りつぶしの層だけが値を持つ"));
        }
        let old = layer.fill_value(channel);
        let was_enabled = layer.is_channel_enabled(channel);
        // 何も変わらない設定は、ロックで断らない（C# の SetFillValue は、変わらないなら先に戻る）
        if old == value && (value.is_none() || was_enabled) {
            return Ok(());
        }
        // 塗りつぶしの値は層の中身: 画像・すべてのロックで断る。アルファが変わるときと、画像のあるチャンネルの値を消す（画像も外れて
        // アルファが変わる）ときは、透明部分のロックでも断る（C# は画像だけを見る。グラデーションを外す側は、値が消える側のアルファの
        // 変化として数える）
        self.ensure_pixels_editable(id, false)?;
        if old.map_or(0, |v| v.a) != value.map_or(0, |v| v.a)
            || value.is_none() && layer.fill_images.contains_key(&channel)
        {
            self.refuse_lock(id, super::LayerLocks::TRANSPARENCY)?;
        }
        // 画像・グラデーションのあるチャンネルの値を消すと、画像・グラデーションも一緒に外れる（1 回の Undo で一緒に戻る）
        if value.is_none()
            && (layer.fill_images.contains_key(&channel)
                || layer.fill_gradients.contains_key(&channel))
        {
            let before = super::effects::FillChannelState::of(layer, channel);
            let mut after = before.clone();
            after.value = None;
            after.image = None;
            after.gradient = None;
            return self.execute(
                Command::FillChannel {
                    id,
                    channel,
                    old: Box::new(before),
                    new: Box::new(after),
                },
                // 費用は値だけを置くときと同じ 64（C# の SetFillValue は、画像・グラデーションが外れても 64）
                64,
            );
        }
        self.record(
            Command::FillValue {
                id,
                channel,
                old,
                new: value,
                was_enabled,
                now_enabled: value.is_some() || was_enabled,
            },
            64,
            coalesce.then_some(CoalesceKey::Fill(id, channel)),
        )
    }

    /// 調整の層の設定を置き換える（種類も変えられる。有効なチャンネルのどれかに使えない設定は断る: 先にそのチャンネルを無効に）。
    pub fn set_adjustment(
        &mut self,
        id: LayerId,
        settings: AdjustmentSettings,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        settings.validate()?;
        let index = self.index_of(id)?;
        let layer = &self.layers[index];
        if layer.kind != LayerKind::Adjustment {
            return Err(CoreError::Unsupported("調整の層だけが調整の設定を持つ"));
        }
        for c in layer.enabled_channels() {
            if !settings.applies_to(self.channel_kind(c)?) {
                return Err(CoreError::Unsupported("有効なチャンネルに使えない調整"));
            }
        }
        self.set_property(
            id,
            Property::Adjustment(settings),
            coalesce.then_some(CoalesceKey::Adjustment(id)),
        )
    }

    // ───────── チャンネル ─────────

    /// 層のチャンネルを有効・無効にする（無効のチャンネルの画素・値は保つ）。ラスターの層で面が無ければ作る（取り消しで外す）。
    /// 調整の層は、その調整を使えないチャンネルを有効にできない。1 回の Undo。
    pub fn set_channel_enabled(
        &mut self,
        id: LayerId,
        channel: Channel,
        enabled: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let kind = self.channel_kind(channel)?;
        let index = self.index_of(id)?;
        let layer = &self.layers[index];
        // 何も変わらない設定は、ロックで断らない（C# の SetChannelEnabled は、変わらないなら先に戻る）
        if layer.is_channel_enabled(channel) == enabled {
            return Ok(());
        }
        self.refuse_lock(id, super::LayerLocks::ALL)?;
        if enabled && layer.kind == LayerKind::Adjustment {
            let applies = layer
                .adjustment
                .as_ref()
                .is_some_and(|a| a.applies_to(kind));
            if !applies {
                return Err(CoreError::Unsupported("色相/彩度は色のチャンネルだけ"));
            }
        }
        // 組（material）を持たないパスだけ、描くチャンネルを無効にできない（C# の `Path.Material == null`。組を持つパスは基準の
        // チャンネルも組のチャンネルも無効にできる。validate_path_target も組があれば基準の有効を要らないとする）
        if !enabled
            && layer
                .paths
                .iter()
                .any(|e| e.path.material().is_none() && e.path.channel() == channel)
        {
            return Err(CoreError::Unsupported(
                "パスで描かれたチャンネルは無効にできない",
            ));
        }
        let had_surface = layer.surface(channel).is_some();
        self.execute(
            Command::ChannelEnabled {
                id,
                channel,
                enabled,
                had_surface,
            },
            64,
        )
    }

    pub(super) fn switch_channel_enabled(
        &mut self,
        id: LayerId,
        channel: Channel,
        enabled: bool,
        had_surface: bool,
        backwards: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(id)?;
        let (w, h, ts) = (self.width, self.height, self.tile_size);
        let layer = &mut self.layers[index];
        if backwards {
            layer.set_enabled(channel, !enabled);
            // 有効にして初めて作った面は、空のままなら外す（空の面が残ると保存の中身が変わる）
            if !had_surface && !layer.is_channel_enabled(channel) {
                if let Some(s) = layer.surface(channel) {
                    if s.tile_count() == 0 {
                        layer.put_surface(channel, None);
                    }
                }
            }
        } else {
            if enabled && layer.kind == LayerKind::Raster && layer.surface(channel).is_none() {
                layer.put_surface(channel, Some(Surface::new(w, h, ts)));
            }
            layer.set_enabled(channel, enabled);
        }
        self.mark_layer(index, Some(channel));
        self.mark_clipped_layers();
        Ok(())
    }

    /// 層のチャンネルの合成モードと不透明度をまとめて置く（空の値ならチャンネルは層に従う）。1 回の Undo。
    pub fn set_channel_blend(
        &mut self,
        id: LayerId,
        channel: Channel,
        blend: ChannelBlend,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        self.require_channel(channel)?;
        let index = self.index_of(id)?;
        if let Some(m) = blend.mode {
            if m == BlendMode::PassThrough && !self.layers[index].is_group() {
                return Err(CoreError::InvalidArgument("PassThrough はグループだけ"));
            }
        }
        if let Some(o) = blend.opacity {
            require_finite(o, "opacity")?;
            if !(0.0..=1.0).contains(&o) {
                return Err(CoreError::InvalidArgument("opacity"));
            }
        }
        self.set_property(
            id,
            Property::ChannelBlend(channel, blend),
            coalesce.then_some(CoalesceKey::ChannelBlend(id, channel)),
        )
    }

    /// 層のチャンネルの合成モードを置く（None で層のモードに従う）。不透明度の設定はそのまま。
    pub fn set_channel_blend_mode(
        &mut self,
        id: LayerId,
        channel: Channel,
        mode: Option<BlendMode>,
    ) -> Result<(), CoreError> {
        let current = self
            .layer(id)
            .ok_or(CoreError::LayerNotFound)?
            .channel_blend(channel);
        self.set_channel_blend(id, channel, ChannelBlend::new(mode, current.opacity), false)
    }

    /// 層のチャンネルの不透明度を置く（None で層の不透明度に従う）。モードの設定はそのまま。coalesce ならドラッグをまとめる。
    pub fn set_channel_opacity(
        &mut self,
        id: LayerId,
        channel: Channel,
        opacity: Option<f64>,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        let current = self
            .layer(id)
            .ok_or(CoreError::LayerNotFound)?
            .channel_blend(channel);
        self.set_channel_blend(
            id,
            channel,
            ChannelBlend::new(current.mode, opacity),
            coalesce,
        )
    }

    // ───────── Normal の出力の設定 ─────────

    /// 文書の Normal の出力の設定（Height → Normal・強さ・端・ファイルの Y）。層の合成は変えない（出力だけ）ので、変化の記録に
    /// タイルを足さない。1 回の Undo（coalesce ならドラッグをまとめる）。
    pub fn set_normal_settings(
        &mut self,
        settings: NormalSettings,
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let old = self.normal_settings;
        if old == settings {
            return Ok(());
        }
        self.record(
            Command::NormalSettings { old, new: settings },
            64,
            coalesce.then_some(CoalesceKey::NormalSettings),
        )
    }
}
