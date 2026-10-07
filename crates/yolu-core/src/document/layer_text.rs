//! テキストレイヤー（あとから編集できる文字）。層の Color の画素は文字の値（[`TextSettings`]）とフォントから描いた結果で、値と画素はいつも
//! 一緒に変わる（1 回の Undo）。フォントの中身は文書に入れないので、描き直す口は呼び手からフォントのバイト列を受け取る。
//!
//! 種類はラスターの層のままで、文字の値を持つことだけが違う（パスの層と同じ形）。手で塗る・画素を動かすなどは次の描き直しで
//! 消えるので断る（`refuse_path_layer`）。値を外す（[`Document::rasterize`]）と今の画素だけが残り、普通に塗れる。パスと文字の値は
//! 同じ層に両方は持てない。

use std::collections::BTreeSet;

use super::material::MaterialCommand;
use super::{CoalesceKey, Command, Document, Target, TileChange};
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::surface::{Surface, Tile};
use crate::text::TextSettings;
use crate::types::{Channel, LayerKind};
use crate::LayerLocks;

/// 文字の値の入れ替え（Undo・Redo）。画素の変化は `pixels` が持つ（値を外すだけなら無い）。
pub(crate) struct TextCommand {
    pub(super) layer: LayerId,
    pub(super) old: Option<TextSettings>,
    pub(super) new: Option<TextSettings>,
    pub(super) pixels: Option<MaterialCommand>,
}

/// 1 回の取り消しに持つ前の画素が、操作の予算に収まるか。
fn check_rollback(changes: &[TileChange], budget: u64) -> Result<(), CoreError> {
    let bytes: u64 = changes
        .iter()
        .map(|c| 64 + c.before.as_ref().map_or(0, Tile::byte_size))
        .sum();
    if bytes > budget {
        Err(CoreError::StrokeBudgetExceeded)
    } else {
        Ok(())
    }
}

/// 値を変える段の費用: 文字の値の前後 + 面の変化（パスの付け替えと同じ数え方）。
fn changed_cost(old: &TextSettings, new: &TextSettings, changes: &[TileChange]) -> u64 {
    state_cost(old)
        + state_cost(new)
        + 64
        + changes
            .iter()
            .map(|c| {
                16 + c.before.as_ref().map_or(0, Tile::byte_size)
                    + c.after.as_ref().map_or(0, Tile::byte_size)
            })
            .sum::<u64>()
}

/// 層を追加する段の費用: 層の追加 128 + 文字の値 + 面（層が画素を持って入るので、元に戻したあとも履歴がその分を持つ）。
fn added_cost(text: &TextSettings, surface: &Surface) -> u64 {
    let mut cost = 128 + state_cost(text) + 64;
    for coord in surface.tile_coords() {
        let tile = surface.tile(coord);
        if !Tile::same(None, tile) {
            cost += 16 + tile.map_or(0, Tile::byte_size);
        }
    }
    cost
}

/// 文字の値が履歴で占める量（文の長さと固定の分）。
fn state_cost(text: &TextSettings) -> u64 {
    128 + text.text.len() as u64
        + match &text.font {
            crate::text::TextFont::Bundled(name) => name.len() as u64,
            crate::text::TextFont::File { path, names, .. } => {
                (path.len() + names.family.len() + names.postscript.len()) as u64 + 40
            }
        }
}

impl Document {
    /// 文字の値から層の画素を描く（文書と同じ大きさ・タイルの面）。
    fn render_text(&self, text: &TextSettings, font: &[u8]) -> Result<Surface, CoreError> {
        crate::text::render(
            text,
            font,
            text.font.index(),
            self.width,
            self.height,
            self.tile_size,
            self.source_budget,
        )
    }

    /// 描いた文字の新しい層を、`above` のすぐ上（同じグループ）か一番上へ追加する。`font` は `text.font` のフォントの中身。
    /// 層の作成・画素・文字の値を 1 回の Undo にする。フォントを読めない・画素の予算を超えるなら層も作らない。`coalesce` なら、続けて
    /// まとめる値の変更（[`Document::set_text`] の `coalesce`）をこの段へまとめる（打ちながら描く間。終わりに `end_coalescing`）。
    pub fn add_text_layer(
        &mut self,
        name: &str,
        text: TextSettings,
        font: &[u8],
        above: Option<LayerId>,
        coalesce: bool,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        text.validate()?;
        let rendered = self.render_text(&text, font)?;
        let id = self.new_layer_id();
        self.ensure_source_growth(rendered.allocated_bytes())?;
        let parent = match above {
            Some(a) => self.layer(a).ok_or(CoreError::LayerNotFound)?.parent,
            None => None,
        };
        self.ensure_new_layer_rewritable(id, parent)?;
        let cost = added_cost(&text, &rendered);
        let mut layer = Layer::new(id, name, LayerKind::Raster);
        layer.put_surface(Channel::Color, Some(rendered));
        layer.set_enabled(Channel::Color, true);
        layer.text = Some(text);
        let id = self.insert_new_costed(layer, above, cost)?;
        if coalesce {
            self.coalesce = Some(CoalesceKey::Text(id));
        }
        Ok(id)
    }

    /// テキストレイヤーの値を変え、画素を描き直す（文・フォント・大きさ・色・位置・回転など、どれでも）。`font` は `text.font` のフォントの中身。
    /// 1 回の Undo。値が同じなら何もしない。テキストレイヤーでない・ロック・予算で断るときは何も変えない。位置・回転を変えるときは
    /// 位置のロックでも断る。`coalesce` なら、直前の段がこの層のまとめている文字の段（`coalesce` で変えた値か、`coalesce` で追加した層）の
    /// とき、その段へまとめる（打ちながら描く間を 1 回の Undo にする。終わりに `end_coalescing`）。
    pub fn set_text(
        &mut self,
        layer: LayerId,
        text: TextSettings,
        font: &[u8],
        coalesce: bool,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        text.validate()?;
        let index = self.index_of(layer)?;
        let old = self.layers[index]
            .text
            .clone()
            .ok_or(CoreError::Unsupported("テキストレイヤーではない"))?;
        if old == text {
            return Ok(());
        }
        // 値は画素を描き直す（アルファも変わる）ので、画像・透明部分・すべてのロックで断る（パスの付け替えと同じ関門）
        self.ensure_pixels_rewritable(layer)?;
        if (old.x, old.y, old.rotation) != (text.x, text.y, text.rotation) {
            self.refuse_lock(layer, LayerLocks::POSITION)?;
        }
        let rendered = self.render_text(&text, font)?;
        let target = Target::Channel(Channel::Color);
        let current = self.layers[index].surface(Channel::Color);
        let coords: BTreeSet<_> = current
            .map(Surface::tile_coords)
            .unwrap_or_default()
            .into_iter()
            .chain(rendered.tile_coords())
            .collect();
        let mut changes = Vec::new();
        for coord in coords {
            let before = current.and_then(|s| s.tile(coord)).cloned();
            let after = rendered.tile(coord).cloned();
            if !Tile::same(before.as_ref(), after.as_ref()) {
                changes.push(TileChange {
                    coord,
                    before,
                    after,
                });
            }
        }
        let key = CoalesceKey::Text(layer);
        if coalesce && self.coalesce == Some(key) && self.redo.is_empty() {
            return self.merge_text(index, text, changes, rendered);
        }
        check_rollback(&changes, self.stroke_budget)?;
        let cost = changed_cost(&old, &text, &changes);
        self.execute(
            Command::Text(TextCommand {
                layer,
                old: Some(old),
                new: Some(text),
                pixels: Some(MaterialCommand {
                    layer,
                    enabled: Vec::new(),
                    parts: vec![(target, changes)],
                }),
            }),
            cost,
        )?;
        if coalesce {
            self.coalesce = Some(key);
        }
        Ok(())
    }

    /// まとめている直前の段（この層の文字の値の段か、この層を追加した段）へ、新しい値と画素の変化をまとめる。直前の段がどちらでも
    /// なければ、何も変えずに断る（まとめの鍵が残っていても段が合わない）。
    fn merge_text(
        &mut self,
        index: usize,
        text: TextSettings,
        changes: Vec<TileChange>,
        rendered: Surface,
    ) -> Result<(), CoreError> {
        let layer = self.layers[index].id;
        let target = Target::Channel(Channel::Color);
        // まとめた後の段の画素の変化と費用を先に作る（予算で断るなら何も変えない）
        let merged = match self.undo.last().map(|e| &e.command) {
            Some(Command::Text(m)) if m.layer == layer && m.pixels.is_some() => {
                let mut list: Vec<TileChange> = m
                    .pixels
                    .iter()
                    .flat_map(|p| p.parts.iter())
                    .filter(|(t, _)| *t == target)
                    .flat_map(|(_, c)| c.iter())
                    .map(|c| TileChange {
                        coord: c.coord,
                        before: c.before.clone(),
                        after: c.after.clone(),
                    })
                    .collect();
                for c in &changes {
                    match list.iter_mut().find(|e| e.coord == c.coord) {
                        Some(e) => e.after = c.after.clone(),
                        None => list.push(TileChange {
                            coord: c.coord,
                            before: c.before.clone(),
                            after: c.after.clone(),
                        }),
                    }
                }
                list.retain(|c| !Tile::same(c.before.as_ref(), c.after.as_ref()));
                check_rollback(&list, self.stroke_budget)?;
                let old = m
                    .old
                    .clone()
                    .ok_or(CoreError::Unsupported("テキストレイヤーではない"))?;
                let cost = changed_cost(&old, &text, &list);
                Some((list, cost))
            }
            Some(Command::Insert {
                index: i,
                len: 1,
                block: None,
            }) if *i == index => None,
            _ => {
                self.end_coalescing();
                return Err(CoreError::Unsupported("まとめるテキストの段が無い"));
            }
        };
        let delta = MaterialCommand {
            layer,
            enabled: Vec::new(),
            parts: vec![(target, changes)],
        };
        self.restore_material(&delta, false)?;
        self.layers[index].text = Some(text.clone());
        let cost = match merged {
            Some((list, cost)) => {
                let top = self.undo.last_mut().expect("確かめた");
                if let Command::Text(m) = &mut top.command {
                    m.new = Some(text);
                    m.pixels = Some(MaterialCommand {
                        layer,
                        enabled: Vec::new(),
                        parts: vec![(target, list)],
                    });
                }
                cost
            }
            // 追加した段: 取り消すと層ごと抜け、やり直すと今の画素と値のまま戻るので、費用だけを今の層に合わせる
            None => added_cost(&text, &rendered),
        };
        let top = self.undo.last_mut().expect("確かめた");
        self.history_bytes = self.history_bytes - top.cost + cost;
        top.cost = cost;
        self.revision += 1;
        Ok(())
    }

    /// 文字の値を外し、今の画素だけを残す（[`Document::rasterize`] から）。画素は変えないので、すべてのロックだけで断る。
    pub(super) fn rasterize_text(
        &mut self,
        layer: LayerId,
        old: TextSettings,
    ) -> Result<(), CoreError> {
        self.refuse_lock(layer, LayerLocks::ALL)?;
        let cost = 64 + state_cost(&old);
        self.execute(
            Command::Text(TextCommand {
                layer,
                old: Some(old),
                new: None,
                pixels: None,
            }),
            cost,
        )
    }

    /// 読み込み用: 履歴なしで文字の値を付ける（画素はもう読み込んである。フォントが無くても今の画素を見せるため、描き直さない）。
    pub fn set_text_for_load(
        &mut self,
        layer: LayerId,
        text: TextSettings,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        text.validate()?;
        let index = self.index_of(layer)?;
        let l = &self.layers[index];
        if l.kind != LayerKind::Raster || l.has_paths() || l.text.is_some() {
            return Err(CoreError::Unsupported(
                "テキストの値を付けられるのはパスとテキストの無いラスターのレイヤーだけ",
            ));
        }
        if l.surface(Channel::Color).is_none() || !l.is_channel_enabled(Channel::Color) {
            return Err(CoreError::Unsupported("テキストレイヤーの Color が無い"));
        }
        self.layers[index].text = Some(text);
        self.external_mutation();
        Ok(())
    }

    pub(super) fn switch_text(
        &mut self,
        m: &mut TextCommand,
        backwards: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(m.layer)?;
        if let Some(pixels) = &m.pixels {
            self.restore_material(pixels, backwards)?;
        }
        self.layers[index].text = if backwards {
            m.old.clone()
        } else {
            m.new.clone()
        };
        Ok(())
    }
}
