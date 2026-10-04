//! 正本（`NativeDocument`）と core の文書の行き来。意味は Unity 版の `DocumentBinary.Read` / `Write` と同じにする（C# が書いた正本は
//! core を通して書き戻すとバイト一致する）。範囲は M2: 層の種類（ラスター・塗りつぶし・調整・グループ）、入れ子と通過・分離、ラスターマスク、
//! クリッピング、チャンネルごとの有効と合成（版 14）、Normal の出力の設定（版 7）、版 22 のユーザーチャンネル。core に無い項目は先に
//! 検査して断り、部分変換を返さない。
use crate::native::{UNITY_NATIVE_VERSION, USER_CHANNELS_VERSION};
use crate::{check, Error, NativeDocument, NativeValue as V, Result, MAX_ENTRY_BYTES};
use std::collections::HashMap;
use yolu_core::{
    AdjustmentSettings, BlendMode, Channel, ChannelBlend, ChannelInfo, ChannelKind, ColorSpace,
    Document, HeightEdgeMode, LayerId, LayerKind, NormalSettings, NormalYDirection, Rgba8,
    TileCoord,
};

fn core_id(mut guid: [u8; 16]) -> u128 {
    guid[..4].reverse();
    guid[4..6].reverse();
    guid[6..8].reverse();
    u128::from_be_bytes(guid)
}
fn native_id(id: u128) -> [u8; 16] {
    let mut guid = id.to_be_bytes();
    guid[..4].reverse();
    guid[4..6].reverse();
    guid[6..8].reverse();
    guid
}

/// 調整の 8 つの値の名前と、使わない値の既定（C# の読み手は種類の作り方で既定に戻す）。
const ADJUSTMENT_PARAMS: [(&str, f64); 8] = [
    ("input_black", 0.0),
    ("input_white", 1.0),
    ("gamma", 1.0),
    ("output_black", 0.0),
    ("output_white", 1.0),
    ("hue", 0.0),
    ("saturation", 0.0),
    ("lightness", 0.0),
];
/// 調整の種類ごとに使う値（`ADJUSTMENT_PARAMS` の番号）。
fn adjustment_uses(kind: i32, param: usize) -> bool {
    match kind {
        1 => param < 5,
        2 => param >= 5,
        _ => false,
    }
}

/// 検証済みの正本の項目をパスで引く。
struct Fields<'a>(HashMap<&'a str, &'a V>);
impl<'a> Fields<'a> {
    fn new(doc: &'a NativeDocument) -> Self {
        Self(
            doc.fields()
                .iter()
                .map(|f| (f.path.as_str(), &f.value))
                .collect(),
        )
    }
    fn get(&self, p: &str) -> Result<&'a V> {
        self.0
            .get(p)
            .copied()
            .ok_or_else(|| Error(format!("正本の項目がありません: {p}")))
    }
    fn wrong(p: &str) -> Error {
        Error(format!("正本の項目の型が違います: {p}"))
    }
    fn int(&self, p: &str) -> Result<i32> {
        match self.get(p)? {
            V::Int(v) => Ok(*v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn byte(&self, p: &str) -> Result<u8> {
        match self.get(p)? {
            V::Byte(v) => Ok(*v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn boolean(&self, p: &str) -> Result<bool> {
        match self.get(p)? {
            V::Bool(v) => Ok(*v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn float(&self, p: &str) -> Result<f64> {
        match self.get(p)? {
            V::Float(v) => Ok(*v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn guid(&self, p: &str) -> Result<[u8; 16]> {
        match self.get(p)? {
            V::Guid(v) => Ok(*v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn text(&self, p: &str) -> Result<&'a str> {
        match self.get(p)? {
            V::Text(v) => Ok(v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn bytes(&self, p: &str) -> Result<&'a [u8]> {
        match self.get(p)? {
            V::Bytes(v) => Ok(v),
            _ => Err(Self::wrong(p)),
        }
    }
    fn channel(&self, p: &str) -> Result<Channel> {
        let c = self.int(p)?;
        usize::try_from(c)
            .ok()
            .and_then(Channel::from_index)
            .ok_or_else(|| Error(format!("{p} のチャンネル {c} は範囲外です")))
    }
    fn rgba(&self, p: &str) -> Result<Rgba8> {
        let b = self.bytes(p)?;
        check(b.len() == 4, format!("{p} は 4 バイトの色ではありません"))?;
        Ok(Rgba8::from_slice(b))
    }
}

/// core に無い項目なら、知らせる単位（層の機能ごとのパス）と理由。知らないパスも断る（読み手に足した項目を黙って通さない）。
fn unsupported(path: &str, fields: &HashMap<&str, &V>) -> Option<(String, &'static str)> {
    let Some(rest) = path.strip_prefix("layers[") else {
        let head = path.split(['.', '[']).next().unwrap_or(path);
        return match head {
            "magic" | "version" | "id" | "width" | "height" | "tile_size" | "normal"
            | "user_channel_count" | "user_channels" | "layer_count" => None,
            "manual_id_colors" => Some(("manual_id_colors".into(), "手動の ID 色")),
            _ => Some((path.into(), "core に無い項目")),
        };
    };
    let Some((index, rest)) = rest.split_once("].") else {
        return Some((path.into(), "core に無い項目"));
    };
    let layer = format!("layers[{index}]");
    let feature = |name: &str, why| Some((format!("{layer}.{name}"), why));
    let mut parts = rest.split('.');
    let head = parts.next().unwrap_or_default();
    match head.split('[').next().unwrap_or_default() {
        "id"
        | "name"
        | "visible"
        | "opacity"
        | "blend"
        | "attributes"
        | "clipping"
        | "channel_blend_count"
        | "channel_blends"
        | "kind"
        | "parent"
        | "fill_count"
        | "fills"
        | "channel_count"
        | "channels"
        | "has_mask"
        | "has_surface_path"
        | "has_filters"
        | "has_canvas_path" => None,
        "mask" => match parts.next().unwrap_or_default().split('[').next() {
            Some("enabled" | "inverted" | "density" | "tile_count" | "tiles") => None,
            Some("filters") => feature("mask.filters", "マスクのフィルター・Generator"),
            Some("anchor") => feature("mask.anchor", "Anchor"),
            _ => Some((path.into(), "core に無い項目")),
        },
        "adjustment" => {
            let leaf = parts.next().unwrap_or_default();
            let Some(param) = ADJUSTMENT_PARAMS.iter().position(|(n, _)| *n == leaf) else {
                return None; // 種類・アルゴリズムの版（読み手が 1 だけを通す）・対象のチャンネル
            };
            let kind = match fields.get(format!("{layer}.adjustment.type").as_str()) {
                Some(V::Int(k)) => *k,
                _ => return Some((path.into(), "調整の種類がありません")),
            };
            let default = ADJUSTMENT_PARAMS[param].1;
            let unused_changed = !adjustment_uses(kind, param)
                && !matches!(fields.get(path), Some(V::Float(v)) if v.to_bits() == default.to_bits());
            unused_changed.then(|| (path.into(), "調整の種類が使わない値が既定ではない"))
        }
        "locks" => feature("locks", "ロック"),
        "image_count" | "images" | "projection" => feature("images", "塗りつぶしの画像・投影"),
        "gradient_count" | "gradients" => feature("gradients", "塗りつぶしのグラデーション"),
        "surface_path" => feature("surface_path", "3D のパス"),
        "canvas_path" => feature("canvas_path", "2D のパス"),
        "filters" => feature("filters", "フィルター・Generator"),
        "anchor_flags" | "anchor" => feature("anchor", "Anchor"),
        _ => Some((path.into(), "core に無い項目")),
    }
}

impl NativeDocument {
    /// core へ渡すと失われる項目（層の機能ごとに 1 つ、`layers[2].filters（フィルター・Generator）` の形）。空なら変換の対象
    /// （タイルの余白・画素の予算は変換時に検査する）。非表示・無効の層やマスクの中の項目も省略せずに断る。
    pub fn core_issues(&self) -> Vec<String> {
        let fields: HashMap<&str, &V> = self
            .fields()
            .iter()
            .map(|f| (f.path.as_str(), &f.value))
            .collect();
        let mut issues = Vec::new();
        for f in self.fields() {
            if let Some((key, why)) = unsupported(&f.path, &fields) {
                let issue = format!("{key}（{why}）");
                if !issues.contains(&issue) {
                    issues.push(issue);
                }
            }
        }
        issues
    }

    /// 編集用の core の文書にする（C# の `DocumentBinary.Read` と同じ意味）。文書と層の ID、透明の画素の RGB を保ち、読み込みを
    /// Undo の履歴に残さない。元の `NativeDocument` は変えない。
    pub fn to_core(&self) -> Result<Document> {
        self.to_core_within(None)
    }

    /// `to_core` の画素の予算を指定できる形（None は core の既定の 256 MiB）。超えたら、どの層のどのタイルで断ったかを添えて断る。
    pub(crate) fn to_core_within(&self, source_budget: Option<u64>) -> Result<Document> {
        let issues = self.core_issues();
        check(
            issues.is_empty(),
            format!("coreへの変換を拒否しました: {}", issues.join("、")),
        )?;
        let f = Fields::new(self);
        let version = self.version();
        let mut doc = Document::with_tile_size(
            self.width() as u32,
            self.height() as u32,
            self.tile_size() as u32,
        )?;
        if let Some(bytes) = source_budget {
            doc.set_source_budget_bytes(bytes)?;
        }
        if version >= 7 {
            let settings = NormalSettings::new(
                f.boolean("normal.derive_from_height")?,
                f.float("normal.strength")?,
                if f.int("normal.edges")? == 1 {
                    HeightEdgeMode::Wrap
                } else {
                    HeightEdgeMode::Clamp
                },
                if f.int("normal.file_direction")? == 1 {
                    NormalYDirection::DirectX
                } else {
                    NormalYDirection::OpenGL
                },
            )?;
            doc.set_normal_settings(settings, false)?;
        }
        if version >= USER_CHANNELS_VERSION {
            for i in 0..f.int("user_channel_count")? {
                let p = format!("user_channels[{i}]");
                let info = ChannelInfo {
                    name: f.text(&format!("{p}.name"))?.to_owned(),
                    kind: match f.int(&format!("{p}.kind"))? {
                        0 => ChannelKind::Color,
                        1 => ChannelKind::Scalar,
                        _ => ChannelKind::Normal,
                    },
                    color_space: if f.int(&format!("{p}.color_space"))? == 0 {
                        ColorSpace::Srgb
                    } else {
                        ColorSpace::Linear
                    },
                    default: f.rgba(&format!("{p}.default"))?,
                };
                doc.insert_channel_for_load(f.channel(&format!("{p}.channel"))?, info)
                    .map_err(|e| Error(format!("{p}をcoreにできません: {e}")))?;
            }
        }
        let mut ids = Vec::with_capacity(self.layer_count());
        let mut parents = Vec::with_capacity(self.layer_count());
        for i in 0..self.layer_count() {
            let p = format!("layers[{i}]");
            load_layer(&mut doc, &f, &p, version).map_err(|e| {
                let name = f.text(&format!("{p}.name")).unwrap_or_default();
                Error(format!("{p}「{name}」をcoreにできません: {e}"))
            })?;
            ids.push(LayerId(core_id(f.guid(&format!("{p}.id"))?)));
            parents.push(if version >= 6 {
                f.guid(&format!("{p}.parent"))?
            } else {
                [0; 16]
            });
        }
        // 親は読み込みの仮の ID で置き、最後に保存した ID へ付け替える（with_persistent_ids が親も写す）
        let temporary: Vec<LayerId> = doc.layers().iter().map(|l| l.id()).collect();
        let by_guid: HashMap<[u8; 16], LayerId> = ids
            .iter()
            .zip(&temporary)
            .map(|(saved, temp)| (native_id(saved.0), *temp))
            .collect();
        let parents: Vec<Option<LayerId>> = parents
            .iter()
            .map(|g| {
                (*g != [0; 16])
                    .then(|| {
                        by_guid
                            .get(g)
                            .copied()
                            .ok_or_else(|| Error("親グループがありません".into()))
                    })
                    .transpose()
            })
            .collect::<Result<_>>()?;
        if parents.iter().any(Option::is_some) {
            doc.set_structure_for_load(&parents)?;
        }
        Ok(doc.with_persistent_ids(core_id(f.guid("id")?), &ids)?)
    }

    /// core の文書から正本を作る（C# の `DocumentBinary.Write` と同じ並び）。ユーザーチャンネルが無ければ Unity 版と同じ版 21、あれば
    /// 版 22。履歴は保存しない。正本の範囲外の寸法・タイル寸法・層の数・名前、進行中のストロークは断る。値の無い塗りつぶしのチャンネルと
    /// グループの有効の印は、合成に効かず C# の書き手も書かないので書かない。
    pub fn from_core(doc: &Document) -> Result<Self> {
        check(!doc.has_active_stroke(), "描画中のストロークがあります")?;
        // 手動の ID の色（正本の版 19）はまだ書けない。黙って落とさず、空でなければ断る
        check(
            doc.id_colors().colors().is_empty(),
            "手動の ID の色はまだ .ylp に書けません",
        )?;
        check(
            doc.width() <= 8192 && doc.height() <= 8192,
            "正本の寸法の上限は8192です",
        )?;
        check(
            (8..=512).contains(&doc.tile_size()) && doc.tile_size().is_power_of_two(),
            "正本のタイル寸法は8〜512の2の累乗です",
        )?;
        check(doc.layers().len() <= 2048, "正本の層数の上限は2048です")?;
        let user: Vec<Channel> = doc
            .channels()
            .into_iter()
            .filter(|c| !c.is_standard())
            .collect();
        let version = if user.is_empty() {
            UNITY_NATIVE_VERSION
        } else {
            USER_CHANNELS_VERSION
        };
        let mut w = Out(Vec::new());
        w.raw(b"DOTPAINT")?;
        w.int(version)?;
        w.raw(&native_id(doc.id()))?;
        for v in [doc.width(), doc.height(), doc.tile_size()] {
            w.int(v as i32)?;
        }
        let normal = doc.normal_settings();
        w.int(NormalSettings::ALGORITHM_VERSION)?;
        w.boolean(normal.derive_from_height())?;
        w.float(normal.strength())?;
        w.int(normal.edges() as i32)?;
        w.int(normal.file_direction() as i32)?;
        if !user.is_empty() {
            w.int(user.len() as i32)?;
            for c in &user {
                let info = doc.channel_info(*c).expect("一覧にある");
                w.int(c.index() as i32)?;
                w.text(&info.name)?;
                w.int(match info.kind {
                    ChannelKind::Color => 0,
                    ChannelKind::Scalar => 1,
                    ChannelKind::Normal => 2,
                })?;
                w.int(match info.color_space {
                    ColorSpace::Srgb => 0,
                    ColorSpace::Linear => 1,
                })?;
                w.raw(&info.default.to_array())?;
            }
        }
        w.int(doc.layers().len() as i32)?;
        for layer in doc.layers() {
            write_layer(&mut w, layer)?;
        }
        Self::read(&w.0)
    }
}

/// 1 つの層を core に足す（C# の読み手と同じ順: 種類で作り、属性、チャンネルごとの合成、画素、マスク）。
fn load_layer(doc: &mut Document, f: &Fields<'_>, p: &str, version: i32) -> Result<()> {
    let name = f.text(&format!("{p}.name"))?;
    let kind = if version >= 3 {
        f.int(&format!("{p}.kind"))?
    } else {
        0
    };
    let id = match kind {
        0 => doc.add_layer(name)?,
        1 => {
            let mut values = Vec::new();
            let mut disabled = Vec::new();
            for k in 0..f.int(&format!("{p}.fill_count"))? {
                let fill = format!("{p}.fills[{k}]");
                let c = f.channel(&format!("{fill}.channel"))?;
                values.push((c, f.rgba(&format!("{fill}.rgba"))?));
                if !f.boolean(&format!("{fill}.enabled"))? {
                    disabled.push(c);
                }
            }
            let id = doc.add_fill_layer(name, &values, None)?;
            for c in disabled {
                doc.set_channel_enabled(id, c, false)?;
            }
            id
        }
        2 => {
            let a = format!("{p}.adjustment");
            let v = |k: usize| f.float(&format!("{a}.{}", ADJUSTMENT_PARAMS[k].0));
            let settings = match f.int(&format!("{a}.type"))? {
                0 => AdjustmentSettings::invert(),
                1 => AdjustmentSettings::levels(v(0)?, v(1)?, v(2)?, v(3)?, v(4)?)?,
                _ => AdjustmentSettings::hue_saturation(v(5)?, v(6)?, v(7)?)?,
            };
            let targets = (0..f.int(&format!("{a}.channel_count"))?)
                .map(|k| f.channel(&format!("{a}.channels[{k}].channel")))
                .collect::<Result<Vec<_>>>()?;
            doc.add_adjustment_layer(name, settings, Some(&targets), None)?
        }
        _ => doc.add_group(name, None)?,
    };
    doc.set_layer_visible(id, f.boolean(&format!("{p}.visible"))?)?;
    doc.set_layer_opacity(id, f.float(&format!("{p}.opacity"))?, false)?;
    let blend = f.int(&format!("{p}.blend"))?;
    doc.set_layer_blend_mode(
        id,
        u8::try_from(blend)
            .ok()
            .and_then(BlendMode::from_index)
            .ok_or_else(|| Error(format!("合成モード {blend} は範囲外です")))?,
    )?;
    let attributes = if version >= 12 {
        f.byte(&format!("{p}.attributes"))?
    } else {
        0
    };
    let clipping = if version >= 12 {
        attributes & 1 != 0
    } else {
        version >= 5 && f.boolean(&format!("{p}.clipping"))?
    };
    doc.set_layer_clipping(id, clipping)?;
    if attributes & 4 != 0 {
        for k in 0..f.byte(&format!("{p}.channel_blend_count"))? {
            let b = format!("{p}.channel_blends[{k}]");
            let parts = f.byte(&format!("{b}.parts"))?;
            let mode = if parts & 1 != 0 {
                let m = f.int(&format!("{b}.mode"))?;
                Some(
                    u8::try_from(m)
                        .ok()
                        .and_then(BlendMode::from_index)
                        .ok_or_else(|| Error(format!("{b}.mode {m} は範囲外です")))?,
                )
            } else {
                None
            };
            let opacity = if parts & 2 != 0 {
                Some(f.float(&format!("{b}.opacity"))?)
            } else {
                None
            };
            doc.set_channel_blend(
                id,
                f.channel(&format!("{b}.channel"))?,
                ChannelBlend::new(mode, opacity),
                false,
            )?;
        }
    }
    let channel_count = if kind == 0 {
        f.int(&format!("{p}.channel_count"))?
    } else {
        0
    };
    for k in 0..channel_count {
        let ch = format!("{p}.channels[{k}]");
        let c = f.channel(&format!("{ch}.channel"))?;
        for t in 0..f.int(&format!("{ch}.tile_count"))? {
            let tile = format!("{ch}.tiles[{t}]");
            let coord = tile_coord(f, &tile)?;
            doc.import_tile(id, c, coord, f.bytes(&format!("{tile}.rgba"))?)
                .map_err(|e| Error(format!("{tile}を変換できません: {e}")))?;
        }
        // 画素の無いチャンネルも面を持つ（C# の GetChannel）。有効の印は保存した値に
        let has_surface = doc.layer(id).is_some_and(|l| l.surface(c).is_some());
        if !has_surface {
            doc.set_channel_enabled(id, c, true)?;
        }
        doc.set_channel_enabled(id, c, f.boolean(&format!("{ch}.enabled"))?)?;
    }
    if version >= 2 && f.boolean(&format!("{p}.has_mask"))? {
        doc.add_layer_mask(id)?;
        doc.set_layer_mask_enabled(id, f.boolean(&format!("{p}.mask.enabled"))?)?;
        doc.set_layer_mask_inverted(id, f.boolean(&format!("{p}.mask.inverted"))?)?;
        doc.set_layer_mask_density(id, f.float(&format!("{p}.mask.density"))?, false)?;
        for t in 0..f.int(&format!("{p}.mask.tile_count"))? {
            let tile = format!("{p}.mask.tiles[{t}]");
            let coord = tile_coord(f, &tile)?;
            doc.import_mask_tile(id, coord, f.bytes(&format!("{tile}.rgba"))?)
                .map_err(|e| Error(format!("{tile}を変換できません: {e}")))?;
        }
    }
    Ok(())
}

fn tile_coord(f: &Fields<'_>, tile: &str) -> Result<TileCoord> {
    let x = f.int(&format!("{tile}.x"))?;
    let y = f.int(&format!("{tile}.y"))?;
    Ok(TileCoord::new(
        u32::try_from(x).map_err(|_| Error(format!("{tile}.x が負です")))?,
        u32::try_from(y).map_err(|_| Error(format!("{tile}.y が負です")))?,
    ))
}

/// 正本のバイト列（512 MiB の予算を書くたびに確かめる）。
struct Out(Vec<u8>);
impl Out {
    fn raw(&mut self, b: &[u8]) -> Result<()> {
        self.0.extend_from_slice(b);
        check(
            self.0.len() <= MAX_ENTRY_BYTES,
            "正本の512 MiB予算を超えています",
        )
    }
    fn int(&mut self, v: i32) -> Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn byte(&mut self, v: u8) -> Result<()> {
        self.raw(&[v])
    }
    fn boolean(&mut self, v: bool) -> Result<()> {
        self.byte(u8::from(v))
    }
    fn float(&mut self, v: f64) -> Result<()> {
        self.raw(&v.to_le_bytes())
    }
    fn text(&mut self, v: &str) -> Result<()> {
        check(v.len() <= 4096, "正本の文字列はUTF-8で4096バイトまでです")?;
        self.int(v.len() as i32)?;
        self.raw(v.as_bytes())
    }
    fn tiles(&mut self, surface: &yolu_core::Surface) -> Result<()> {
        self.int(surface.tile_count() as i32)?;
        let mut tile = vec![0; surface.tile_bytes()];
        for coord in surface.tile_coords() {
            surface.copy_tile(coord, &mut tile)?;
            self.int(coord.x as i32)?;
            self.int(coord.y as i32)?;
            self.int(tile.len() as i32)?;
            self.raw(&tile)?;
        }
        Ok(())
    }
}

/// 1 つの層（C# の `DocumentBinary.Write` の層の並び。M2 に無いロック・画像・グラデーション・パス・フィルター・Anchor は書かない）。
fn write_layer(w: &mut Out, layer: &yolu_core::Layer) -> Result<()> {
    let named = |why: &str| format!("層「{}」の{why}", layer.name());
    w.raw(&native_id(layer.id().0))?;
    w.text(layer.name())
        .map_err(|e| Error(named(&format!("名前: {e}"))))?;
    w.boolean(layer.visible())?;
    w.float(layer.opacity())?;
    w.int(layer.blend_mode() as i32)?;
    let blends: Vec<(Channel, ChannelBlend)> = layer.channel_blends().collect();
    w.byte(u8::from(layer.clipping()) | if blends.is_empty() { 0 } else { 4 })?;
    if !blends.is_empty() {
        w.byte(blends.len() as u8)?;
        for (c, b) in &blends {
            w.int(c.index() as i32)?;
            w.byte(u8::from(b.mode.is_some()) | if b.opacity.is_some() { 2 } else { 0 })?;
            if let Some(m) = b.mode {
                w.int(m as i32)?;
            }
            if let Some(o) = b.opacity {
                w.float(o)?;
            }
        }
    }
    w.int(layer.kind() as i32)?;
    w.raw(&layer.parent().map_or([0; 16], |p| native_id(p.0)))?;
    let fills: Vec<(Channel, Rgba8)> = layer.fill_values().collect();
    w.int(fills.len() as i32)?;
    for (c, v) in fills {
        w.int(c.index() as i32)?;
        w.boolean(layer.is_channel_enabled(c))?;
        w.raw(&v.to_array())?;
    }
    if layer.kind() == LayerKind::Adjustment {
        let a = layer
            .adjustment()
            .ok_or_else(|| Error(named("調整の設定がありません")))?;
        w.int(a.kind() as i32)?;
        w.int(AdjustmentSettings::ALGORITHM_VERSION)?;
        for v in [
            a.input_black(),
            a.input_white(),
            a.gamma(),
            a.output_black(),
            a.output_white(),
            a.hue(),
            a.saturation(),
            a.lightness(),
        ] {
            w.float(v)?;
        }
        let enabled = layer.enabled_channels();
        w.int(enabled.len() as i32)?;
        for c in enabled {
            w.int(c.index() as i32)?;
        }
    }
    let surfaces = layer.surface_channels();
    if layer.kind() == LayerKind::Raster {
        let orphan = layer
            .enabled_channels()
            .into_iter()
            .find(|c| layer.surface(*c).is_none());
        check(
            orphan.is_none(),
            named(&format!("{orphan:?} は面が無いのに有効です")),
        )?;
    } else {
        check(
            surfaces.is_empty(),
            named("画素はラスターの層だけが持てます"),
        )?;
    }
    w.int(surfaces.len() as i32)?;
    for c in surfaces {
        w.int(c.index() as i32)?;
        w.boolean(layer.is_channel_enabled(c))?;
        w.tiles(layer.surface(c).expect("面のあるチャンネル"))?;
    }
    w.boolean(layer.mask().is_some())?;
    if let Some(mask) = layer.mask() {
        w.boolean(mask.enabled())?;
        w.boolean(mask.inverted())?;
        w.float(mask.density())?;
        w.tiles(mask.surface())?;
    }
    // 3D のパス・フィルター・2D のパスは core に無い
    for _ in 0..3 {
        w.boolean(false)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> NativeDocument {
        NativeDocument::read(
            &std::fs::read(format!(
                "{}/tests/fixtures/{name}.utpaint",
                env!("CARGO_MANIFEST_DIR")
            ))
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn pixel_budget_refuses_with_the_layer_and_tile_and_keeps_the_original() {
        let native = fixture("m2-masks");
        let whole = native.to_core().unwrap().allocated_bytes();
        assert!(whole > 4096, "{whole}");
        // 予算ちょうどなら通り、1 バイト足りなければ断る（画素もマスクも数える）
        assert!(native.to_core_within(Some(whole)).is_ok());
        for budget in [whole - 1, whole / 2, 1, 0] {
            let message = native
                .to_core_within(Some(budget))
                .err()
                .unwrap()
                .to_string();
            assert!(message.contains("layers["), "{message}");
            assert!(message.contains("予算"), "{message}");
        }
        assert_eq!(
            NativeDocument::read(&native.to_bytes()).unwrap().to_bytes(),
            native.to_bytes()
        );
    }

    #[test]
    fn fill_and_group_layers_cost_no_pixels() {
        // 1 画素の文書は、ラスターの 1 タイル（8×8×4 バイト）だけが画素。塗りつぶし・グループは予算に数えない
        let native = fixture("m2-tiny");
        assert_eq!(native.to_core().unwrap().allocated_bytes(), 256);
        assert!(native.to_core_within(Some(256)).is_ok());
        assert!(native.to_core_within(Some(255)).is_err());
    }
}
