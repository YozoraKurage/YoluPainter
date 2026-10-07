use super::material::MaterialCommand;
use super::selection::{fill_pixel, mask_fill_pixel};
use super::*;
use crate::material::{ChannelPaint, GradientSettings};
use crate::SelectionMask;

impl Document {
    /// 全チャンネルを同じ範囲へ塗る。無効のチャンネルは有効にし、取消・失敗・変更なしなら戻す。画像・すべてのロックと、消すときの
    /// 透明部分のロックで断る（何も変えない）。透明部分のロックでは全チャンネルがアルファを守る。
    pub fn fill_material(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        opacity: f64,
        region: Option<&SelectionMask>,
        erase: bool,
    ) -> Result<bool, CoreError> {
        if !(0.0..=1.0).contains(&opacity) {
            return Err(CoreError::InvalidArgument("opacity"));
        }
        self.edit_material_region(
            layer,
            channels,
            region,
            erase,
            |m, _, _, start, amount, keep_alpha| {
                fill_pixel(start, m.value, opacity, amount, erase, keep_alpha)
            },
        )
    }
    /// 各チャンネルの値から透明へ。同じチャンネルの終点値を渡すと二つのマテリアルを補間する。
    pub fn gradient_material(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        to: Option<&[ChannelPaint]>,
        gradient: &GradientSettings,
        region: Option<&SelectionMask>,
        erase: bool,
    ) -> Result<bool, CoreError> {
        gradient.validate()?;
        self.validate_material(channels)?;
        if let Some(to) = to {
            self.validate_material(to)?;
            if channels.len() != to.len()
                || channels
                    .iter()
                    .any(|a| !to.iter().any(|b| a.channel == b.channel))
            {
                return Err(CoreError::InvalidArgument("グラデーション両端のチャンネル"));
            }
        }
        self.edit_material_region(
            layer,
            channels,
            region,
            erase,
            |m, x, y, start, amount, keep_alpha| {
                let g = GradientSettings {
                    from: m.value,
                    to: to
                        .and_then(|t| t.iter().find(|t| t.channel == m.channel))
                        .map_or(Rgba8::TRANSPARENT, |m| m.value),
                    ..*gradient
                };
                fill_pixel(
                    start,
                    g.color_at(x as f64 + 0.5, y as f64 + 0.5),
                    g.opacity,
                    amount,
                    erase,
                    keep_alpha,
                )
            },
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn gradient(
        &mut self,
        layer: LayerId,
        channel: Channel,
        gradient: &GradientSettings,
        region: Option<&SelectionMask>,
        erase: bool,
    ) -> Result<bool, CoreError> {
        gradient.validate()?;
        let index = self.index_of(layer)?;
        if !self.layers[index].is_channel_enabled(channel) {
            return Err(CoreError::Unsupported("無効のチャンネルには塗れない"));
        }
        self.gradient_material(
            layer,
            &[ChannelPaint::new(channel, gradient.from)],
            Some(&[ChannelPaint::new(channel, gradient.to)]),
            gradient,
            region,
            erase,
        )
    }
    /// マスクの隠す量をグラデーションのアルファで増減する。すべてのロックだけで断る（画像・透明部分のロックはマスクを妨げない）。
    pub fn gradient_mask(
        &mut self,
        layer: LayerId,
        gradient: &GradientSettings,
        region: Option<&SelectionMask>,
        reveal: bool,
    ) -> Result<bool, CoreError> {
        gradient.validate()?;
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        if self.layers[index].mask.is_none() {
            return Err(CoreError::Unsupported("レイヤーにマスクが無い"));
        }
        self.refuse_lock(layer, LayerLocks::ALL)?;
        let effective = self.effective_region(region)?;
        let coords: Vec<_> = effective
            .as_ref()
            .map_or_else(|| self.canvas_tiles().collect(), |r| r.tile_coords());
        let changes = self.edit_region_tiles(
            index,
            Target::Mask,
            effective.as_ref(),
            &coords,
            &mut 0,
            |x, y, start, amount| {
                let coverage =
                    amount * gradient.color_at(x as f64 + 0.5, y as f64 + 0.5).a as f64 / 255.0;
                mask_fill_pixel(start, gradient.opacity, coverage, reveal)
            },
        )?;
        if changes.is_empty() {
            return Ok(false);
        }
        for c in &changes {
            self.mark_target_tile(index, Target::Mask, c.coord);
        }
        self.revision += 1;
        self.push_material(MaterialCommand {
            layer,
            enabled: Vec::new(),
            parts: vec![(Target::Mask, changes)],
        });
        Ok(true)
    }
    /// pixel の最後の引数は書き込みの関門が決めた透明部分のロック（`keep_alpha`）で、`fill_pixel` へそのまま渡す。
    fn edit_material_region<F>(
        &mut self,
        layer: LayerId,
        channels: &[ChannelPaint],
        region: Option<&SelectionMask>,
        erase: bool,
        pixel: F,
    ) -> Result<bool, CoreError>
    where
        F: Fn(&ChannelPaint, u32, u32, Rgba8, f64, bool) -> Rgba8 + Sync,
    {
        self.ensure_no_stroke()?;
        self.validate_material(channels)?;
        let index = self.index_of(layer)?;
        self.ensure_raster(index)?;
        // 無効のチャンネルを有効にする前に断る（断った塗りが何も残さない）
        let keep_alpha = self.pixel_write_guard(layer, erase)?;
        self.refuse_path_layer(index)?;
        let effective = self.effective_region(region)?;
        let coords: Vec<_> = effective
            .as_ref()
            .map_or_else(|| self.canvas_tiles().collect(), |r| r.tile_coords());
        let mut command = MaterialCommand {
            layer,
            enabled: Vec::new(),
            parts: Vec::new(),
        };
        for c in channels {
            if !self.layers[index].is_channel_enabled(c.channel) {
                let had = self.layers[index].surface(c.channel).is_some();
                self.switch_channel_enabled(layer, c.channel, true, had, false)?;
                command.enabled.push((c.channel, had));
            }
            self.ensure_surface(index, c.channel);
        }
        let mut rollback = 0;
        for c in channels {
            let target = Target::Channel(c.channel);
            match self.edit_region_tiles(
                index,
                target,
                effective.as_ref(),
                &coords,
                &mut rollback,
                |x, y, start, amount| pixel(c, x, y, start, amount, keep_alpha),
            ) {
                Ok(changes) => {
                    if !changes.is_empty() {
                        command.parts.push((target, changes));
                    }
                }
                Err(e) => {
                    self.restore_material(&command, true)
                        .expect("巻き戻しは元の予算内");
                    return Err(e);
                }
            }
        }
        if command.parts.is_empty() {
            self.restore_material(&command, true)?;
            return Ok(false);
        }
        for (target, changes) in &command.parts {
            for c in changes {
                self.mark_target_tile(index, *target, c.coord);
            }
        }
        self.revision += 1;
        self.push_material(command);
        Ok(true)
    }
}
