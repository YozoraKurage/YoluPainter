//! 編集できるパスを持つ層（C# の `PaintDocument.Paths`）。層の対象チャンネルの画素はパスから描いた結果で、パスと画素はいつも
//! 一緒に変わる（1 回の Undo）。3D のパスを描くのは呼び手（モデルの面が要る）で、ここは描いた結果の面とパスを受け取って入れ替える。
//! 2D のパスは [`Document::set_canvas_path`] がここで描く。
//!
//! 手で塗る・塗りつぶす・マテリアルで塗ると次の描き直しで消えるので、パスの層には断る。パスを外す（[`Document::rasterize`]）と
//! 今の画素だけが残り、普通に塗れる。

use std::collections::BTreeSet;

use super::material::MaterialCommand;
use super::{Command, Document, Entry, Target, TileChange};
use crate::effects::{paths_error, LayerPath};
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::paths::{render_canvas, Options};
use crate::surface::{Surface, Tile};
use crate::types::{Channel, LayerKind};

/// パスの状態の入れ替え（Undo・Redo）。画素の変化は material が持つ（パスの種類によっては無い）。
pub(crate) struct PathCommand {
    pub(super) layer: LayerId,
    pub(super) old: Option<LayerPath>,
    pub(super) new: Option<LayerPath>,
    pub(super) pixels: Option<MaterialCommand>,
}

impl Document {
    /// パスを持つ層に手で描く・塗る操作を断る（次の描き直しで消える）。
    pub(super) fn refuse_path_layer(&self, index: usize) -> Result<(), CoreError> {
        if self.layers[index].path.is_some() {
            Err(CoreError::Unsupported("パスで描かれた層には手で描けない"))
        } else {
            Ok(())
        }
    }

    fn validate_path_target(&self, index: usize, path: &LayerPath) -> Result<(), CoreError> {
        let l = &self.layers[index];
        if l.kind != LayerKind::Raster {
            return Err(CoreError::Unsupported("パスで描けるのはラスターの層だけ"));
        }
        for c in path.channels() {
            self.require_channel(c)?;
        }
        if path.material().is_none() && !l.is_channel_enabled(path.channel()) {
            return Err(CoreError::Unsupported("パスのチャンネルが層で有効でない"));
        }
        if let Some(old) = &l.path {
            if old.channel() != path.channel() {
                return Err(CoreError::Unsupported("層のパスはチャンネルを変えない"));
            }
            if old.is_canvas() != path.is_canvas() {
                return Err(CoreError::Unsupported(
                    "層のパスは種類（モデルの上かキャンバスの上か）を変えない",
                ));
            }
        }
        // パスは層の画素を描き直す（アルファも変わる）ので、画像・透明部分・すべてのロックで断る。型と状態の検査のあと（C# の
        // ValidatePathTarget の末尾）
        self.ensure_pixels_rewritable(l.id)
    }

    /// 層にパスを付ける（差し替える）。層のパスのチャンネルの画素を、描いた結果 `rendered`（パスの組のチャンネルごとに 1 つ。画布と
    /// 同じ大きさの面）へそっくり入れ替える。組から外れた古いパスのチャンネルは空にする。ほかのチャンネルとマスクは変えない。
    /// 層で有効でないチャンネルは有効にする。1 回の Undo。予算（画素・巻き戻し）を超えるなら何も変えずに断る。
    pub fn set_path(
        &mut self,
        layer: LayerId,
        path: LayerPath,
        rendered: Vec<(Channel, Surface)>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        path.validate()?;
        let index = self.index_of(layer)?;
        self.validate_path_target(index, &path)?;
        let paints = path.channels();
        let mut rendered: std::collections::BTreeMap<Channel, Surface> = {
            let mut map = std::collections::BTreeMap::new();
            for (c, s) in rendered {
                if map.insert(c, s).is_some() {
                    return Err(CoreError::InvalidArgument(
                        "描いた面のチャンネルが重なっている",
                    ));
                }
            }
            map
        };
        if rendered.len() != paints.len()
            || paints.iter().any(|c| {
                rendered.get(c).is_none_or(|s| {
                    s.width() != self.width
                        || s.height() != self.height
                        || s.tile_size() != self.tile_size
                })
            })
        {
            return Err(CoreError::InvalidArgument(
                "描いた面は、パスのチャンネルごとに、文書と同じ大きさで 1 つ",
            ));
        }
        let old_path = self.layers[index].path.clone();
        let old_channels: Vec<Channel> = old_path
            .as_ref()
            .map(LayerPath::channels)
            .unwrap_or_default();
        let channels: BTreeSet<Channel> = paints.iter().chain(&old_channels).copied().collect();
        // 層で有効でないチャンネルを有効にする（取り消しで戻す）
        let mut enabled: Vec<(Channel, bool)> = Vec::new();
        for c in &paints {
            if !self.layers[index].is_channel_enabled(*c) && !old_channels.contains(c) {
                let had = self.layers[index].surface(*c).is_some();
                self.switch_channel_enabled(layer, *c, true, had, false)?;
                enabled.push((*c, had));
            }
        }
        let undo_enabling = |doc: &mut Document, enabled: &[(Channel, bool)]| {
            for (c, had) in enabled.iter().rev() {
                let _ = doc.switch_channel_enabled(layer, *c, true, *had, true);
            }
        };
        let mut parts: Vec<(Target, Vec<TileChange>)> = Vec::new();
        let mut rollback = 0u64;
        let result: Result<(), CoreError> = (|| {
            for c in &channels {
                let target = Target::Channel(*c);
                self.ensure_surface(index, *c);
                let replacement = rendered.remove(c);
                let growth = self.growth_for(index, target);
                let mut changes = Vec::new();
                let coords: BTreeSet<_> = {
                    let surface = self.layers[index].surface(*c).expect("作った");
                    surface
                        .tile_coords()
                        .into_iter()
                        .chain(replacement.iter().flat_map(Surface::tile_coords))
                        .collect()
                };
                for coord in coords {
                    let surface = self.layers[index].surface_mut(*c).expect("作った");
                    let before = surface.tile(coord).cloned();
                    let after = replacement.as_ref().and_then(|r| r.tile(coord).cloned());
                    if Tile::same(before.as_ref(), after.as_ref()) {
                        continue;
                    }
                    rollback += 64 + before.as_ref().map_or(0, Tile::byte_size);
                    if rollback > self.stroke_budget {
                        parts.push((target, changes));
                        return Err(CoreError::StrokeBudgetExceeded);
                    }
                    let delta = surface.growth_to(coord, after.as_ref());
                    if let Err(e) = growth.ensure(surface.allocated_bytes(), delta) {
                        parts.push((target, changes));
                        return Err(e);
                    }
                    surface.restore(coord, after.as_ref());
                    changes.push(TileChange {
                        coord,
                        before,
                        after,
                    });
                }
                parts.push((target, changes));
            }
            Ok(())
        })();
        if let Err(e) = result {
            for (target, changes) in parts.iter().rev() {
                for c in changes.iter().rev() {
                    if let Some(s) = self.target_surface_mut(index, *target) {
                        s.restore(c.coord, c.before.as_ref());
                    }
                }
            }
            undo_enabling(self, &enabled);
            for c in &channels {
                self.mark_layer(index, Some(*c));
            }
            return Err(e);
        }
        for (target, changes) in &parts {
            for c in changes {
                self.mark_target_tile(index, *target, c.coord);
            }
        }
        self.layers[index].path = Some(path.clone());
        self.revision += 1;
        // 履歴の費用は C# の SetPath と同じ（有効にしたチャンネルごとに 64 + パスの状態 + チャンネルごとの面の変化）
        let cost = 64 * enabled.len() as u64
            + path.state_cost()
            + parts
                .iter()
                .map(|(_, p)| {
                    64 + p
                        .iter()
                        .map(|c| {
                            16 + c.before.as_ref().map_or(0, Tile::byte_size)
                                + c.after.as_ref().map_or(0, Tile::byte_size)
                        })
                        .sum::<u64>()
                })
                .sum::<u64>();
        self.push(Entry {
            command: Command::Path(PathCommand {
                layer,
                old: old_path,
                new: Some(path),
                pixels: Some(MaterialCommand {
                    layer,
                    enabled,
                    parts,
                }),
            }),
            cost,
        });
        Ok(())
    }

    /// 描いたパスの新しい層を、`above` のすぐ上（同じグループ）か一番上へ足す。層の作成・チャンネルの有効・画素・パスを 1 回の Undo にする。
    /// 画素の予算を超えるなら層も作らない。
    pub fn add_path_layer(
        &mut self,
        name: &str,
        path: LayerPath,
        rendered: Vec<(Channel, Surface)>,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        path.validate()?;
        let paints = path.channels();
        for c in &paints {
            self.require_channel(*c)?;
        }
        if rendered.len() != paints.len()
            || paints.iter().any(|c| {
                rendered.iter().filter(|(rc, _)| rc == c).count() != 1
                    || rendered.iter().any(|(rc, s)| {
                        rc == c
                            && (s.width() != self.width
                                || s.height() != self.height
                                || s.tile_size() != self.tile_size)
                    })
            })
        {
            return Err(CoreError::InvalidArgument(
                "描いた面は、パスのチャンネルごとに、文書と同じ大きさで 1 つ",
            ));
        }
        let id = self.new_layer_id();
        // 親のグループのロックは入る層にも効く（C# は層を作ってから SetPath の関門を通る）。画素の予算を先に確かめる点も同じ
        let bytes: u64 = rendered.iter().map(|(_, s)| s.allocated_bytes()).sum();
        self.ensure_source_growth(bytes)?;
        let parent = match above {
            Some(a) => self.layer(a).ok_or(CoreError::LayerNotFound)?.parent,
            None => None,
        };
        self.ensure_new_layer_rewritable(id, parent)?;
        // 履歴の費用は C# の AddPathLayer（層の追加 128 + Color 以外のチャンネルを有効にする 64 ずつ + パスの状態 + チャンネルごとの
        // 面の変化）と同じ: 層が画素を持って入るので、元に戻したあとも履歴がその分を持つ
        let mut cost = 128 + path.state_cost();
        for (c, surface) in &rendered {
            if *c != Channel::Color {
                cost += 64;
            }
            cost += 64;
            for coord in surface.tile_coords() {
                let tile = surface.tile(coord);
                if !Tile::same(None, tile) {
                    cost += 16 + tile.map_or(0, Tile::byte_size);
                }
            }
        }
        let mut layer = Layer::new(id, name, LayerKind::Raster);
        layer.put_surface(
            Channel::Color,
            Some(Surface::new(self.width, self.height, self.tile_size)),
        );
        layer.set_enabled(Channel::Color, true);
        for (c, surface) in rendered {
            layer.put_surface(c, Some(surface));
            layer.set_enabled(c, true);
        }
        layer.path = Some(path);
        self.insert_new_costed(layer, above, cost)
    }

    /// 2D のパスを描いて層に付ける（描き直しも）。選択に依らない。評価は文書の予算で行い、1 回の Undo。
    pub fn set_canvas_path(
        &mut self,
        layer: LayerId,
        path: crate::paths::CanvasPath,
    ) -> Result<(), CoreError> {
        self.set_canvas_path_cancellable(layer, path, None)
    }

    /// `set_canvas_path` の、評価を取り消せる形（cancel が立つと `Cancelled`。何も変えない）。
    pub fn set_canvas_path_cancellable(
        &mut self,
        layer: LayerId,
        path: crate::paths::CanvasPath,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let path = LayerPath::Canvas(path);
        path.validate()?;
        let index = self.index_of(layer)?;
        self.validate_path_target(index, &path)?;
        let LayerPath::Canvas(canvas) = &path else {
            unreachable!("作った")
        };
        let options = Options {
            width: self.width,
            height: self.height,
            tile_size: self.tile_size,
            source_budget_bytes: self.source_budget,
            stroke_budget_bytes: self.stroke_budget,
            cancel,
            ..Options::default()
        };
        let rendered = render_canvas(canvas, &options).map_err(paths_error)?;
        self.set_path(layer, path, rendered.channels)
    }

    /// パスを外し、今の画素だけを残す（その後は普通に塗れる）。1 回の Undo。パスが無ければ何もしない。
    pub fn rasterize(&mut self, layer: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        let Some(old) = self.layers[index].path.clone() else {
            return Ok(());
        };
        // 画素は変えないので、すべてのロックだけで断る（C# の Rasterize）
        self.refuse_lock(layer, super::LayerLocks::ALL)?;
        self.execute(
            Command::Path(PathCommand {
                layer,
                old: Some(old),
                new: None,
                pixels: None,
            }),
            64,
        )
    }

    /// 読み込み用: 履歴なしでパスを付ける（画素はもう読み込んである。パスのチャンネルの面があることだけ確かめる）。
    pub fn set_path_for_load(&mut self, layer: LayerId, path: LayerPath) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        path.validate()?;
        let index = self.index_of(layer)?;
        let l = &self.layers[index];
        if l.kind != LayerKind::Raster || l.path.is_some() {
            return Err(CoreError::Unsupported(
                "パスを付けられるのはパスの無いラスターの層だけ",
            ));
        }
        for c in path.channels() {
            if l.surface(c).is_none() {
                return Err(CoreError::Unsupported("パスのチャンネルの面が層に無い"));
            }
        }
        if path.material().is_none() && !l.is_channel_enabled(path.channel()) {
            return Err(CoreError::Unsupported("パスのチャンネルが層で有効でない"));
        }
        self.layers[index].path = Some(path);
        self.external_mutation();
        Ok(())
    }

    pub(super) fn switch_path(
        &mut self,
        m: &mut PathCommand,
        backwards: bool,
    ) -> Result<(), CoreError> {
        let index = self.index_of(m.layer)?;
        if let Some(pixels) = &m.pixels {
            self.restore_material(pixels, backwards)?;
        }
        self.layers[index].path = if backwards {
            m.old.clone()
        } else {
            m.new.clone()
        };
        Ok(())
    }
}
