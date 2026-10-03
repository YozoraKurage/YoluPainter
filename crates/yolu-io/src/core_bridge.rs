//! M1のラスターColorだけを変換する。未対応の値は先に検査し、部分変換を返さない。
use crate::{check, Error, NativeDocument, NativeValue as V, Result, MAX_ENTRY_BYTES};
use std::collections::HashMap;
use yolu_core::{BlendMode, Channel, Document, LayerId, TileCoord};

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
impl NativeDocument {
    /// coreへ渡すと失われる項目。空ならM1の変換対象（タイル余白・メモリ予算は変換時にも検査）。
    /// 無効なマスク・非表示の非ラスター層なども省略せず拒否する。
    pub fn core_issues(&self) -> Vec<String> {
        self.fields()
            .iter()
            .filter_map(|f| {
                let supported = match f.path.as_str() {
                    "magic" | "version" | "id" | "width" | "height" | "tile_size"
                    | "layer_count" => true,
                    "normal.algorithm" => f.value == V::Int(1),
                    "normal.derive_from_height" => f.value == V::Bool(false),
                    "normal.strength" => f.value == V::Float(4.0),
                    "normal.edges" | "normal.file_direction" => f.value == V::Int(0),
                    p if p.starts_with("layers[") => {
                        let Some((_, rest)) = p.split_once("].") else {
                            return Some(format!("未対応項目: {p}"));
                        };
                        match rest {
                            "id" | "name" | "visible" | "opacity" | "blend" | "clipping" => true,
                            "attributes" => matches!(f.value, V::Byte(bits) if bits & !1 == 0),
                            "kind" | "fill_count" => f.value == V::Int(0),
                            "parent" => f.value == V::Guid([0; 16]),
                            "channel_count" => matches!(f.value, V::Int(0..=1)),
                            "has_mask" | "has_surface_path" | "has_canvas_path" | "has_filters" => {
                                f.value == V::Bool(false)
                            }
                            "channels[0].channel" => f.value == V::Int(0),
                            "channels[0].enabled" => f.value == V::Bool(true),
                            "channels[0].tile_count" => true,
                            p if p.starts_with("channels[0].tiles[") => {
                                p.rsplit_once("].").is_some_and(|(_, leaf)| {
                                    matches!(leaf, "x" | "y" | "length" | "rgba")
                                })
                            }
                            _ => false,
                        }
                    }
                    _ => false,
                };
                (!supported).then(|| format!("M1で保持できない項目: {}", f.path))
            })
            .collect()
    }

    /// ラスター・有効なColorチャンネルだけの文書を編集用coreへ変換する。
    /// 文書と層のID、透明画素のRGBも保持する。元のNativeDocumentは変更しない。
    pub fn to_core(&self) -> Result<Document> {
        let issues = self.core_issues();
        check(
            issues.is_empty(),
            format!("coreへの変換を拒否しました: {}", issues.join("、")),
        )?;
        let fields: HashMap<_, _> = self
            .fields()
            .iter()
            .map(|f| (f.path.as_str(), &f.value))
            .collect();
        let int = |p: &str| match fields.get(p) {
            Some(V::Int(v)) => *v,
            _ => unreachable!("検証済みの整数: {p}"),
        };
        let guid = |p: &str| match fields.get(p) {
            Some(V::Guid(v)) => core_id(*v),
            _ => unreachable!("検証済みのID: {p}"),
        };
        let mut doc = Document::with_tile_size(
            self.width() as u32,
            self.height() as u32,
            self.tile_size() as u32,
        )?;
        let mut ids = Vec::new();
        for i in 0..self.layer_count() {
            let p = format!("layers[{i}]");
            let V::Text(name) = fields[format!("{p}.name").as_str()] else {
                unreachable!()
            };
            let id = doc.add_layer(name)?;
            ids.push(LayerId(guid(&format!("{p}.id"))));
            if let V::Bool(v) = fields[format!("{p}.visible").as_str()] {
                doc.set_layer_visible(id, *v)?;
            }
            if let V::Float(v) = fields[format!("{p}.opacity").as_str()] {
                doc.set_layer_opacity(id, *v, false)?;
            }
            doc.set_layer_blend_mode(
                id,
                BlendMode::from_index(int(&format!("{p}.blend")) as u8).unwrap(),
            )?;
            let clipped = match fields.get(format!("{p}.attributes").as_str()) {
                Some(V::Byte(v)) => v & 1 != 0,
                _ => fields.get(format!("{p}.clipping").as_str()) == Some(&&V::Bool(true)),
            };
            doc.set_layer_clipping(id, clipped)?;
            if int(&format!("{p}.channel_count")) == 1 {
                let channel = format!("{p}.channels[0]");
                for t in 0..int(&format!("{channel}.tile_count")) {
                    let tile = format!("{channel}.tiles[{t}]");
                    let coord = TileCoord::new(
                        int(&format!("{tile}.x")) as u32,
                        int(&format!("{tile}.y")) as u32,
                    );
                    let V::Bytes(rgba) = fields[format!("{tile}.rgba").as_str()] else {
                        unreachable!()
                    };
                    doc.import_tile(id, Channel::Color, coord, rgba)
                        .map_err(|e| Error(format!("{tile}を変換できません: {e}")))?;
                }
            }
        }
        Ok(doc.with_persistent_ids(guid("id"), &ids)?)
    }

    /// core文書から正本21を作る。履歴は保存しない。M1以外のチャンネルや
    /// 正本の範囲外の寸法・タイル寸法、進行中のストロークは拒否する。
    pub fn from_core(doc: &Document) -> Result<Self> {
        check(
            !doc.has_active_stroke(),
            "描画中のストロークを確定または取り消してから保存してください",
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
        let mut out = Vec::new();
        let mut write = |v: V| -> Result<()> {
            v.write(&mut out);
            check(
                out.len() <= MAX_ENTRY_BYTES,
                "正本の512 MiB予算を超えています",
            )
        };
        for value in [
            V::Bytes(std::sync::Arc::from(b"DOTPAINT".as_slice())),
            V::Int(21),
            V::Guid(native_id(doc.id())),
            V::Int(doc.width() as i32),
            V::Int(doc.height() as i32),
            V::Int(doc.tile_size() as i32),
            V::Int(1),
            V::Bool(false),
            V::Float(4.0),
            V::Int(0),
            V::Int(0),
            V::Int(doc.layers().len() as i32),
        ] {
            write(value)?;
        }
        for layer in doc.layers() {
            for channel in Channel::ALL.into_iter().filter(|c| *c != Channel::Color) {
                check(
                    layer.surface(channel).is_none() && !layer.is_channel_enabled(channel),
                    format!("層「{}」の{channel:?}はM1変換の対象外です", layer.name()),
                )?;
            }
            check(
                layer.is_channel_enabled(Channel::Color),
                "無効なColorチャンネルはM1変換の対象外です",
            )?;
            let surface = layer
                .surface(Channel::Color)
                .ok_or_else(|| Error("Colorの面がありません".into()))?;
            for value in [
                V::Guid(native_id(layer.id().0)),
                V::Text(layer.name().into()),
                V::Bool(layer.visible()),
                V::Float(layer.opacity()),
                V::Int(layer.blend_mode() as i32),
                V::Byte(u8::from(layer.clipping())),
                V::Int(0),
                V::Guid([0; 16]),
                V::Int(0),
                V::Int(1),
                V::Int(0),
                V::Bool(true),
                V::Int(surface.tile_count() as i32),
            ] {
                write(value)?;
            }
            let mut tile = vec![0; surface.tile_bytes()];
            for coord in surface.tile_coords() {
                surface.copy_tile(coord, &mut tile)?;
                for value in [
                    V::Int(coord.x as i32),
                    V::Int(coord.y as i32),
                    V::Int(tile.len() as i32),
                    V::Bytes(std::sync::Arc::from(tile.as_slice())),
                ] {
                    write(value)?;
                }
            }
            for _ in 0..4 {
                write(V::Bool(false))?;
            }
        }
        Self::read(&out)
    }
}
