//! 編集できるパスを持つレイヤー（C# の `PaintDocument.Paths`）。レイヤーの対象チャンネルの画素はパスの一覧から描いた結果で、一覧と画素は
//! いつも一緒に変わる（1 回の Undo）。3D のパスを描くのは呼び手（モデルの面が要る）で、ここは描いた結果の面と一覧を受け取って
//! 入れ替える。2D のパスは [`Document::set_canvas_paths`] がここで描く。
//!
//! 手で塗る・塗りつぶす・マテリアルで塗ると次の描き直しで消えるので、パスレイヤーには断る。パスを外す（[`Document::rasterize`]）と
//! 今の画素だけが残り、普通に塗れる。

use std::collections::BTreeSet;

use super::material::MaterialCommand;
use super::{Command, Document, Entry, Target, TileChange};
use crate::effects::{paths_error, LayerPath};
use crate::error::CoreError;
use crate::layer::{Layer, LayerId};
use crate::paths::{list_channels, render_list, validate_list, LayerPathEntry, Options};
use crate::surface::{Surface, Tile};
use crate::types::{Channel, LayerKind};

/// パスの一覧の入れ替え（Undo・Redo）。画素の変化は material が持つ（ラスタライズには無い）。
pub(crate) struct PathCommand {
    pub(super) layer: LayerId,
    pub(super) old: Vec<LayerPathEntry>,
    pub(super) new: Vec<LayerPathEntry>,
    pub(super) pixels: Option<MaterialCommand>,
}

/// 一覧の履歴の大きさ（パスごとの状態と名前）。
fn list_cost(entries: &[LayerPathEntry]) -> u64 {
    entries.iter().map(LayerPathEntry::state_cost).sum()
}

/// 1 本のパスを、レイヤーの今の一覧の 1 本目の名前と表示を引き継いで一覧にする（ID が同じときだけ引き継ぐ）。
fn single(old: &[LayerPathEntry], path: LayerPath) -> Vec<LayerPathEntry> {
    let mut entry = LayerPathEntry::new(path);
    if let Some(first) = old.first().filter(|f| f.id() == entry.id()) {
        entry.name = first.name.clone();
        entry.visible = first.visible;
    }
    vec![entry]
}

impl Document {
    /// パスを持つレイヤーに手で描く・塗る操作を断る（次の描き直しで消える）。
    pub(super) fn refuse_path_layer(&self, index: usize) -> Result<(), CoreError> {
        if self.layers[index].has_paths() {
            Err(CoreError::Unsupported(
                "パスで描かれたレイヤーには手で描けない",
            ))
        } else if self.layers[index].text.is_some() {
            Err(CoreError::Unsupported("テキストレイヤーには手で描けない"))
        } else {
            Ok(())
        }
    }

    fn validate_paths_target(
        &self,
        index: usize,
        entries: &[LayerPathEntry],
    ) -> Result<(), CoreError> {
        let l = &self.layers[index];
        // 塗りつぶしレイヤーのパスは、塗りつぶしと効果のスタックの結果の上に重ねる（評価の最後）
        if !matches!(l.kind, LayerKind::Raster | LayerKind::Fill) {
            return Err(CoreError::Unsupported(
                "パスで描けるのはラスターと塗りつぶしレイヤーだけ",
            ));
        }
        if l.text.is_some() {
            return Err(CoreError::Unsupported(
                "テキストレイヤーにはパスを付けられない",
            ));
        }
        validate_list(entries).map_err(paths_error)?;
        for e in entries {
            for c in e.path.channels() {
                self.require_channel(c)?;
            }
            // ラスターレイヤーは、描くチャンネルを有効にしてから（C# と同じ）。塗りつぶしレイヤーは値の無いチャンネルにも描くので、
            // パスを付けるときに有効にする
            if l.kind == LayerKind::Raster
                && e.path.material().is_none()
                && !l.is_channel_enabled(e.path.channel())
            {
                return Err(CoreError::Unsupported(
                    "パスのチャンネルがレイヤーで有効でない",
                ));
            }
        }
        if let (Some(old), Some(new)) = (l.paths.first(), entries.first()) {
            if old.path.channel() != new.path.channel() {
                return Err(CoreError::Unsupported(
                    "レイヤーのパスはチャンネルを変えない",
                ));
            }
            if old.path.is_canvas() != new.path.is_canvas() {
                return Err(CoreError::Unsupported(
                    "レイヤーのパスは種類（モデルの上かキャンバスの上か）を変えない",
                ));
            }
        }
        // パスはレイヤーの画素を描き直す（アルファも変わる）ので、画像・透明部分・すべてのロックで断る。型と状態の検査のあと（C# の
        // ValidatePathTarget の末尾）
        self.ensure_pixels_rewritable(l.id)
    }

    /// レイヤーにパスを 1 本だけ付ける（一覧をこの 1 本にする）。今の 1 本目と同じ ID なら名前と表示を引き継ぐ。ほかは [`Document::set_paths`]。
    pub fn set_path(
        &mut self,
        layer: LayerId,
        path: LayerPath,
        rendered: Vec<(Channel, Surface)>,
    ) -> Result<(), CoreError> {
        let index = self.index_of(layer)?;
        let entries = single(&self.layers[index].paths, path);
        self.set_paths(layer, entries, rendered)
    }

    /// レイヤーのパスの一覧を入れ替える。レイヤーの一覧のチャンネル（[`list_channels`]。前の一覧のものも）の画素を、描いた結果 `rendered`（新しい
    /// 一覧のチャンネルごとに 1 つ。キャンバスと同じ大きさの面）へそっくり入れ替える。新しい一覧から外れたチャンネルは空にする。ほかの
    /// チャンネルとマスクは変えない。レイヤーで有効でないチャンネルは有効にする。空の一覧はパスを外して、そのチャンネルを空にする。
    /// 1 回の Undo。予算（画素・巻き戻し）を超えるなら何も変えずに断る。
    pub fn set_paths(
        &mut self,
        layer: LayerId,
        entries: Vec<LayerPathEntry>,
        rendered: Vec<(Channel, Surface)>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        validate_list(&entries).map_err(paths_error)?;
        let index = self.index_of(layer)?;
        self.validate_paths_target(index, &entries)?;
        let paints = list_channels(&entries);
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
                "描いた面は、パスのチャンネルごとに、キャンバスと同じ大きさで 1 つ",
            ));
        }
        let old_paths = self.layers[index].paths.clone();
        let old_channels: Vec<Channel> = list_channels(&old_paths);
        let channels: BTreeSet<Channel> = paints.iter().chain(&old_channels).copied().collect();
        // レイヤーで有効でないチャンネルを有効にする（取り消しで戻す）
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
        self.layers[index].paths = entries.clone();
        self.revision += 1;
        // 履歴の費用は C# の SetPath と同じ（有効にしたチャンネルごとに 64 + パスの状態 + チャンネルごとの面の変化）。一覧はパスごとの
        // 状態と名前の和
        let cost = 64 * enabled.len() as u64
            + list_cost(&entries)
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
            kind: super::HistoryKind::Path,
            command: Command::Path(PathCommand {
                layer,
                old: old_paths,
                new: entries,
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

    /// 描いたパスの新しいレイヤーを、`above` のすぐ上（同じグループ）か一番上へ足す。レイヤーの作成・チャンネルの有効・画素・パスを 1 回の Undo にする。
    /// 画素の予算を超えるならレイヤーも作らない。
    pub fn add_path_layer(
        &mut self,
        name: &str,
        path: LayerPath,
        rendered: Vec<(Channel, Surface)>,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.add_paths_layer(name, vec![LayerPathEntry::new(path)], rendered, above)
    }

    /// パスの一覧（1 本以上）を描いた新しいレイヤーを足す（[`Document::add_path_layer`] の一覧の形）。
    pub fn add_paths_layer(
        &mut self,
        name: &str,
        entries: Vec<LayerPathEntry>,
        rendered: Vec<(Channel, Surface)>,
        above: Option<LayerId>,
    ) -> Result<LayerId, CoreError> {
        self.ensure_no_stroke()?;
        if entries.is_empty() {
            return Err(CoreError::InvalidArgument(
                "パスレイヤーにはパスが 1 本以上要る",
            ));
        }
        validate_list(&entries).map_err(paths_error)?;
        let paints = list_channels(&entries);
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
                "描いた面は、パスのチャンネルごとに、キャンバスと同じ大きさで 1 つ",
            ));
        }
        let id = self.new_layer_id();
        // 親のグループのロックは入るレイヤーにも効く（C# はレイヤーを作ってから SetPath の関門を通る）。画素の予算を先に確かめる点も同じ
        let bytes: u64 = rendered.iter().map(|(_, s)| s.allocated_bytes()).sum();
        self.ensure_source_growth(bytes)?;
        let parent = match above {
            Some(a) => self.layer(a).ok_or(CoreError::LayerNotFound)?.parent,
            None => None,
        };
        self.ensure_new_layer_rewritable(id, parent)?;
        // 履歴の費用は C# の AddPathLayer（レイヤーの追加 128 + Color 以外のチャンネルを有効にする 64 ずつ + パスの状態 + チャンネルごとの
        // 面の変化）と同じ: レイヤーが画素を持って入るので、元に戻したあとも履歴がその分を持つ
        let mut cost = 128 + list_cost(&entries);
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
        layer.paths = entries;
        self.insert_new_costed(layer, above, cost)
    }

    /// 2D のパスを描いてレイヤーに付ける（一覧をこの 1 本にする。描き直しも）。選択に依らない。評価は文書の予算で行い、1 回の Undo。
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
        let index = self.index_of(layer)?;
        let entries = single(&self.layers[index].paths, LayerPath::Canvas(path));
        self.set_canvas_paths_cancellable(layer, entries, cancel)
    }

    /// 2D のパスの一覧を描いてレイヤーに付ける（描き直しも）。1 回の Undo。
    pub fn set_canvas_paths(
        &mut self,
        layer: LayerId,
        entries: Vec<LayerPathEntry>,
    ) -> Result<(), CoreError> {
        self.set_canvas_paths_cancellable(layer, entries, None)
    }

    /// `set_canvas_paths` の、評価を取り消せる形。3D のパスは断る（描くには形が要る）。
    pub fn set_canvas_paths_cancellable(
        &mut self,
        layer: LayerId,
        entries: Vec<LayerPathEntry>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        if entries.iter().any(|e| !e.path.is_canvas()) {
            return Err(CoreError::InvalidArgument(
                "3D のパスは呼び手が形で描いて set_paths へ渡す",
            ));
        }
        validate_list(&entries).map_err(paths_error)?;
        let index = self.index_of(layer)?;
        self.validate_paths_target(index, &entries)?;
        let options = Options {
            width: self.width,
            height: self.height,
            tile_size: self.tile_size,
            source_budget_bytes: self.source_budget,
            stroke_budget_bytes: self.stroke_budget,
            cancel,
            images: self.effects.inputs.images.clone(),
            ..Options::default()
        };
        let rendered = render_list(&entries, None, &options).map_err(paths_error)?;
        self.set_paths(layer, entries, rendered.channels)
    }

    /// パス（テキストレイヤーならテキストの値）を外し、今の画素だけを残す（その後は普通に塗れる）。1 回の Undo。どちらも無ければ何もしない。
    pub fn rasterize(&mut self, layer: LayerId) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        if let Some(text) = self.layers[index].text.clone() {
            return self.rasterize_text(layer, text);
        }
        let old = self.layers[index].paths.clone();
        if old.is_empty() {
            return Ok(());
        }
        // 塗りつぶしレイヤーは画素を持たない（パスの画素は評価の最後に重ねるだけ）ので、パスを外すと絵が消える。画素にはしない
        if self.layers[index].kind == LayerKind::Fill {
            return Err(CoreError::Unsupported(
                "塗りつぶしレイヤーのパスは画素にできない",
            ));
        }
        // 画素は変えないので、すべてのロックだけで断る（C# の Rasterize）
        self.refuse_lock(layer, super::LayerLocks::ALL)?;
        self.execute(
            Command::Path(PathCommand {
                layer,
                old,
                new: Vec::new(),
                pixels: None,
            }),
            64,
        )
    }

    /// 一覧のパス 1 本の名前だけを替える。画素は変えないので描き直さない（3D のパスでもモデルを要らない）。1 回の Undo。
    /// 名前が今と同じなら何もしない。名前の決まり（128 文字・制御文字なし）に合わなければ断る。画素は変えないので、すべてのロックだけで
    /// 断る（[`Document::rasterize`] と同じ）。
    pub fn rename_path(&mut self, layer: LayerId, path: u128, name: &str) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        let index = self.index_of(layer)?;
        let old = self.layers[index].paths.clone();
        let Some(i) = old.iter().position(|e| e.id() == path) else {
            return Err(CoreError::InvalidArgument(
                "名前を替えるパスがレイヤーに無い",
            ));
        };
        if old[i].name == name {
            return Ok(());
        }
        let mut new = old.clone();
        new[i].name = name.to_string();
        new[i].validate().map_err(paths_error)?;
        self.refuse_lock(layer, super::LayerLocks::ALL)?;
        let cost = 64 + list_cost(&new);
        self.execute(
            Command::Path(PathCommand {
                layer,
                old,
                new,
                pixels: None,
            }),
            cost,
        )
    }

    /// 読み込み用: 塗りつぶしレイヤーのパスの画素の面を作る（タイルの無い面も。パスの一覧を付ける前に）。
    pub fn ensure_fill_path_surface(
        &mut self,
        id: LayerId,
        channel: Channel,
    ) -> Result<(), CoreError> {
        self.ensure_loadable()?;
        self.require_channel(channel)?;
        let index = self.index_of(id)?;
        if self.layers[index].kind != LayerKind::Fill {
            return Err(CoreError::Unsupported(
                "塗りつぶしレイヤーのパスの画素は塗りつぶしレイヤーだけ",
            ));
        }
        if self.ensure_surface(index, channel) {
            self.external_mutation();
        }
        Ok(())
    }

    /// 読み込み用: 塗りつぶしレイヤーのパスの画素の 1 タイル（[`Document::import_tile`] の塗りつぶしレイヤーの形。読み込みなので履歴を消す）。
    pub fn import_fill_path_tile(
        &mut self,
        id: LayerId,
        channel: Channel,
        coord: crate::types::TileCoord,
        bytes: &[u8],
    ) -> Result<bool, CoreError> {
        self.ensure_fill_path_surface(id, channel)?;
        let index = self.index_of(id)?;
        let growth = self.growth_for(index, Target::Channel(channel));
        let changed = self.layers[index]
            .surface_mut(channel)
            .expect("作った")
            .import_tile(coord, bytes, growth)?;
        if changed {
            self.mark_target_tile(index, Target::Channel(channel), coord);
            self.external_mutation();
        }
        Ok(changed)
    }

    /// 読み込み用: 履歴なしでパスを付ける（画素はもう読み込んである。パスのチャンネルの面があることだけ確かめる）。
    pub fn set_path_for_load(&mut self, layer: LayerId, path: LayerPath) -> Result<(), CoreError> {
        self.set_paths_for_load(layer, vec![LayerPathEntry::new(path)])
    }

    /// 読み込み用: 履歴なしでパスの一覧を付ける（1 本以上。画素はもう読み込んである）。
    pub fn set_paths_for_load(
        &mut self,
        layer: LayerId,
        entries: Vec<LayerPathEntry>,
    ) -> Result<(), CoreError> {
        self.ensure_no_stroke()?;
        validate_list(&entries).map_err(paths_error)?;
        let index = self.index_of(layer)?;
        let l = &self.layers[index];
        if !matches!(l.kind, LayerKind::Raster | LayerKind::Fill)
            || l.has_paths()
            || l.text.is_some()
            || entries.is_empty()
        {
            return Err(CoreError::Unsupported(
                "パスを付けられるのはパスの無いラスターか塗りつぶしレイヤーだけ",
            ));
        }
        for e in &entries {
            for c in e.path.channels() {
                if l.surface(c).is_none() {
                    return Err(CoreError::Unsupported(
                        "パスのチャンネルの面がレイヤーに無い",
                    ));
                }
            }
            if e.path.material().is_none() && !l.is_channel_enabled(e.path.channel()) {
                return Err(CoreError::Unsupported(
                    "パスのチャンネルがレイヤーで有効でない",
                ));
            }
        }
        self.layers[index].paths = entries;
        self.external_mutation();
        Ok(())
    }

    /// パスの作業面（`paths` の評価が持つ私の文書）のレイヤーのチャンネルへ、1 画素を「通常」で重ねる（straight のアルファで
    /// `src · a + dst · da · (1 − a)`）。リボンの画素ごとの色に使う。履歴には積まない。
    pub(crate) fn paths_blend_pixel(
        &mut self,
        layer: LayerId,
        channel: Channel,
        x: u32,
        y: u32,
        src: crate::Rgba8,
        alpha: f64,
    ) -> Result<(), CoreError> {
        let a = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            0.0
        };
        if a <= 0.0 {
            return Ok(());
        }
        let index = self.index_of(layer)?;
        self.ensure_surface(index, channel);
        let growth = self.growth_for(index, Target::Channel(channel));
        let surface = self.layers[index].surface_mut(channel).expect("作った");
        let dst = surface.pixel(x, y)?;
        let da = dst.a as f64 / 255.0;
        let out = a + da * (1.0 - a);
        let mix = |s: u8, d: u8| {
            ((s as f64 * a + d as f64 * da * (1.0 - a)) / out)
                .round()
                .clamp(0.0, 255.0) as u8
        };
        let color = crate::Rgba8::new(
            mix(src.r, dst.r),
            mix(src.g, dst.g),
            mix(src.b, dst.b),
            (out * 255.0).round().clamp(0.0, 255.0) as u8,
        );
        surface.set_pixel(x, y, color, growth)?;
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
        self.layers[index].paths = if backwards {
            m.old.clone()
        } else {
            m.new.clone()
        };
        Ok(())
    }
}
