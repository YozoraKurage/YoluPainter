//! M1のラスターColor文書への明示変換。名前によるレイヤー対応付けはしない。
use super::*;
use crate::{check, Error, Result};
use std::collections::HashSet;
use yolu_core::{Channel, Document as CoreDocument, LayerId, TileCoord};
impl ReadResult {
    pub fn to_core(&self) -> Result<CoreDocument> {
        check(
            self.mode == CompatibilityMode::EditableRaster,
            "PSD 原本は編集できません",
        )?;
        self.document
            .as_ref()
            .ok_or_else(|| Error("編集用文書がありません".into()))?
            .to_core()
    }
}
impl Document {
    pub fn core_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for (i, l) in self.layers.iter().enumerate() {
            let p = format!("layers[{i}]");
            if l.kind != LayerKind::Raster {
                issues.push(format!("{p}: M1はラスター以外を保持できません"))
            }
            if l.mask.is_some() {
                issues.push(format!("{p}.mask: M1はマスクを保持できません"))
            }
            if l.locks != 0 {
                issues.push(format!("{p}.locks: M1はロックを保持できません"))
            }
            if l.left < 0
                || l.top < 0
                || i64::from(l.left) + i64::from(l.width) > i64::from(self.width)
                || i64::from(l.top) + i64::from(l.height) > i64::from(self.height)
            {
                issues.push(format!("{p}: キャンバス外の画素を切り捨てられません"))
            }
        }
        issues
    }
    pub fn to_core(&self) -> Result<CoreDocument> {
        super::write::validate(self, &Limits::default())?;
        let issues = self.core_issues();
        check(
            issues.is_empty(),
            format!("core 変換を拒否しました: {}", issues.join("、")),
        )?;
        let mut d = CoreDocument::new(self.width, self.height)?;
        let salt = d.id() & ((1u128 << 64) - 1);
        let mut ids = Vec::new();
        let ts = d.tile_size();
        for l in self.layers.iter().rev() {
            let id = d.add_layer(&l.name)?;
            ids.push(LayerId(((l.id as u128) << 96) | salt));
            d.set_layer_visible(id, l.visible)?;
            d.set_layer_opacity(id, f64::from(l.opacity) / 255.0, false)?;
            d.set_layer_blend_mode(
                id,
                yolu_core::BlendMode::from_index(l.blend_mode as u8).unwrap(),
            )?;
            d.set_layer_clipping(id, l.clipping)?;
            let x0 = l.left as u32;
            let y0 = self.height - l.top as u32 - l.height;
            let x1 = x0 + l.width;
            let y1 = y0 + l.height;
            for ty in y0 / ts..y1.div_ceil(ts) {
                for tx in x0 / ts..x1.div_ceil(ts) {
                    let mut tile = vec![0; (ts * ts * 4) as usize];
                    for y in (ty * ts).max(y0)..((ty + 1) * ts).min(y1) {
                        for x in (tx * ts).max(x0)..((tx + 1) * ts).min(x1) {
                            let src = ((self.height - 1 - y - l.top as u32) * l.width + x - x0)
                                as usize
                                * 4;
                            let dest = ((y % ts) * ts + x % ts) as usize * 4;
                            tile[dest..dest + 4].copy_from_slice(&l.pixels_rgba[src..src + 4])
                        }
                    }
                    d.import_tile(id, Channel::Color, TileCoord::new(tx, ty), &tile)?;
                }
            }
        }
        let doc_id = d.id();
        Ok(d.with_persistent_ids(doc_id, &ids)?)
    }
    /// PSDへの新規投影。インポート原本の編集保存には、この結果と `write_edited` を使う。
    pub fn from_core(d: &CoreDocument) -> Result<Self> {
        check(
            !d.has_active_stroke(),
            "ストロークを確定・取消してからPSDを書き出してください",
        )?;
        let limits = Limits::default();
        check(
            d.width() <= limits.max_dimension
                && d.height() <= limits.max_dimension
                && u64::from(d.width()) * u64::from(d.height()) <= limits.max_canvas_pixels,
            "PSD キャンバス予算超過",
        )?;
        check(
            !d.layers().is_empty() && d.layers().len() <= limits.max_layers,
            "PSD レイヤー数の予算超過",
        )?;
        let mut layers = Vec::new();
        let mut used = HashSet::new();
        let mut budget = 0u64;
        for l in d.layers().iter().rev() {
            for ch in Channel::ALL {
                if ch != Channel::Color {
                    check(
                        l.surface(ch).is_none() && !l.is_channel_enabled(ch),
                        format!("層「{}」の{ch:?}はM1 PSD変換の対象外です", l.name()),
                    )?
                }
            }
            check(
                l.is_channel_enabled(Channel::Color),
                "無効なColorはM1 PSD変換の対象外です",
            )?;
            let surface = l
                .surface(Channel::Color)
                .ok_or_else(|| Error("Color面がありません".into()))?;
            let (mut left, mut bottom, mut right, mut top) = (d.width(), d.height(), 0, 0);
            for c in surface.tile_coords() {
                left = left.min(c.x * d.tile_size());
                bottom = bottom.min(c.y * d.tile_size());
                right = d.width().min(right.max((c.x + 1) * d.tile_size()));
                top = d.height().min(top.max((c.y + 1) * d.tile_size()))
            }
            if right <= left || top <= bottom {
                left = 0;
                bottom = 0;
                right = 1;
                top = 1
            }
            let width = right - left;
            let height = top - bottom;
            budget += u64::from(width) * u64::from(height) * 4;
            check(budget <= 128 * 1024 * 1024, "PSD 投影の128 MiB画素予算超過")?;
            let mut pixels = vec![0; width as usize * height as usize * 4];
            for y in 0..height {
                for x in 0..width {
                    let p = (y as usize * width as usize + x as usize) * 4;
                    pixels[p..p + 4]
                        .copy_from_slice(&surface.pixel(left + x, top - 1 - y)?.to_array())
                }
            }
            let mut id = ((l.id().0 >> 96) as i32) & i32::MAX;
            if id == 0 {
                id = 1
            }
            while !used.insert(id) {
                id = if id == i32::MAX { 1 } else { id + 1 }
            }
            layers.push(Layer {
                id,
                name: l.name().into(),
                left: left as i32,
                top: (d.height() - top) as i32,
                width,
                height,
                opacity: (l.opacity() * 255.0).round_ties_even() as u8,
                visible: l.visible(),
                blend_mode: BlendMode::ALL[l.blend_mode() as usize],
                clipping: l.clipping(),
                pixels_rgba: pixels,
                ..Layer::default()
            });
        }
        let rgba = d.composite(d.bounds())?;
        let mut top_down = Vec::with_capacity(rgba.len());
        for row in rgba.chunks_exact(d.width() as usize * 4).rev() {
            top_down.extend(row)
        }
        let out = Self {
            width: d.width(),
            height: d.height(),
            layers,
            composite_rgba: Some(top_down),
        };
        super::write::validate(&out, &limits)?;
        Ok(out)
    }
}
