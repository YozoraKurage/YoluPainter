use crate::{check, guid, is_hash, Error, Result, MAX_ENTRY_BYTES};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// Unity 版（0.2.0 の `DocumentBinary`）が書き、読める正本の一番新しい版。
pub const UNITY_NATIVE_VERSION: i32 = 21;
/// 文書のユーザーチャンネル（core の 6〜63）の一覧を足した版。ユーザーチャンネルのある文書だけがこの版になり、
/// Unity 版の読み手は「Unsupported archive version」で断る（形式と決めは README の「ユーザーチャンネル（正本の版 22）」）。
pub const USER_CHANNELS_VERSION: i32 = 22;
/// この読み手が読める一番新しい版。
pub const MAX_NATIVE_VERSION: i32 = USER_CHANNELS_VERSION;
/// 標準のチャンネルの数（番号 0〜5。Unity 版の PaintChannel）。
const STANDARD_CHANNELS: i32 = 6;
/// 版 22 のユーザーチャンネル（番号 → 種類: 0 色・1 スカラー・2 法線）。版 21 までは空。
type UserChannels = BTreeMap<i32, i32>;

/// 正本の値。f64 は演算せずビットを保存し、RGBA・GUID の並びも変えない。
#[derive(Clone, Debug, PartialEq)]
pub enum NativeValue {
    Int(i32),
    Byte(u8),
    Bool(bool),
    Float(f64),
    Guid([u8; 16]),
    Text(String),
    Bytes(Arc<[u8]>),
}
impl NativeValue {
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Int(v) => out.extend(v.to_le_bytes()),
            Self::Byte(v) => out.push(*v),
            Self::Bool(v) => out.push(u8::from(*v)),
            Self::Float(v) => out.extend(v.to_le_bytes()),
            Self::Guid(v) => out.extend(v),
            Self::Text(v) => {
                out.extend((v.len() as i32).to_le_bytes());
                out.extend(v.as_bytes());
            }
            Self::Bytes(v) => out.extend(v.as_ref()),
        }
    }
}
/// `layers[0].channels[0].tiles[0].rgba` などのパスでアクセスする、ディスク順の値。
#[derive(Clone, Debug, PartialEq)]
pub struct NativeField {
    pub path: String,
    pub value: NativeValue,
}
#[derive(Clone, Debug)]
pub struct NativeDocument {
    version: i32,
    id: String,
    width: i32,
    height: i32,
    tile_size: i32,
    layers: usize,
    fields: Vec<NativeField>,
}
impl NativeDocument {
    pub fn version(&self) -> i32 {
        self.version
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn width(&self) -> i32 {
        self.width
    }
    pub fn height(&self) -> i32 {
        self.height
    }
    pub fn tile_size(&self) -> i32 {
        self.tile_size
    }
    pub fn layer_count(&self) -> usize {
        self.layers
    }
    pub fn fields(&self) -> &[NativeField] {
        &self.fields
    }
    pub fn field(&self, path: &str) -> Option<&NativeValue> {
        self.fields
            .iter()
            .find(|f| f.path == path)
            .map(|f| &f.value)
    }
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for f in &self.fields {
            f.value.write(&mut out);
        }
        out
    }
    /// 値を差し替えて全体を再検証する。未知の構造や不整合を保存に持ち越さない。
    pub fn with_value(&self, path: &str, value: NativeValue) -> Result<Self> {
        let mut fields = self.fields.clone();
        let field = fields
            .iter_mut()
            .find(|f| f.path == path)
            .ok_or_else(|| Error(format!("正本の項目がありません: {path}")))?;
        check(
            std::mem::discriminant(&field.value) == std::mem::discriminant(&value),
            "正本の値の型を変更できません",
        )?;
        field.value = value;
        let mut out = Vec::new();
        for f in fields {
            f.value.write(&mut out);
        }
        Self::read(&out)
    }
    pub fn read(b: &[u8]) -> Result<Self> {
        check(
            b.len() <= MAX_ENTRY_BYTES,
            "正本の512 MiB予算を超えています",
        )?;
        let mut r = Reader {
            bytes: b,
            at: 0,
            prefix: String::new(),
            fields: Vec::new(),
        };
        check(
            r.blob("magic", 8)?.as_ref() == b"DOTPAINT",
            "正本の識別子が不正です",
        )?;
        let version = r.int("version", 1, MAX_NATIVE_VERSION)?;
        let id = guid(&r.id("id", false)?);
        let width = r.int("width", 1, 8192)?;
        let height = r.int("height", 1, 8192)?;
        let ts = r.int("tile_size", 8, 512)?;
        check(ts & (ts - 1) == 0, "タイル寸法が2の累乗ではありません")?;
        if version >= 7 {
            r.block("normal", |r| {
                r.int("algorithm", 1, 1)?;
                r.boolean("derive_from_height")?;
                r.float("strength", -256., 256.)?;
                r.int("edges", 0, 1)?;
                r.int("file_direction", 0, 1)?;
                Ok(())
            })?;
        }
        let user = if version >= USER_CHANNELS_VERSION {
            user_channels(&mut r)?
        } else {
            UserChannels::new()
        };
        let count = r.int("layer_count", 0, 2048)?;
        let mut layers = Vec::new();
        let mut ids = HashSet::new();
        let mut anchor_ids = HashMap::new();
        for i in 0..count {
            let l = r.block(&format!("layers[{i}]"), |r| {
                layer(r, version, width, height, ts, &user)
            })?;
            check(ids.insert(l.id), "レイヤーIDが重複しています")?;
            for id in &l.anchors {
                check(
                    anchor_ids.insert(*id, i).is_none(),
                    "Anchor IDが重複しています",
                )?;
            }
            layers.push(l);
        }
        if version >= 19 && r.at < b.len() {
            r.block("manual_id_colors", |r| {
                check(
                    r.blob("tag", 4)?.as_ref() == b"YLID",
                    "末尾に未知のデータがあります",
                )?;
                let n = r.int("count", 1, 4096)?;
                check(is_hash(&r.string("binding")?), "手動ID色の指紋が不正です")?;
                let mut prev = -1;
                for i in 0..n {
                    r.block(&format!("colors[{i}]"), |r| {
                        let p = r.int("part", 0, 3999999)?;
                        check(p > prev, "手動ID色の部品番号の並びが不正です")?;
                        prev = p;
                        r.int("rgb", 0, 0xffffff)?;
                        Ok(())
                    })?;
                }
                Ok(())
            })?;
        }
        check(
            r.at == b.len(),
            "正本の末尾に未知のデータがあります。新しい読み手が必要です",
        )?;
        let by_id: HashMap<_, _> = layers.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
        for (i, l) in layers.iter().enumerate() {
            let mut parent = l.parent;
            let mut ancestors = HashSet::new();
            while parent != [0; 16] {
                let p = *by_id
                    .get(&parent)
                    .ok_or_else(|| Error("親グループがありません".into()))?;
                check(
                    p > i && layers[p].kind == 3 && ancestors.insert(p),
                    "親グループの位置・種類・循環が不正です",
                )?;
                for child in layers.iter().take(p).skip(i + 1) {
                    let mut id = child.parent;
                    let mut hops = 0;
                    while id != parent && id != [0; 16] && hops < layers.len() {
                        id = by_id.get(&id).map(|&k| layers[k].parent).unwrap_or([0; 16]);
                        hops += 1;
                    }
                    check(id == parent, "グループの子が連続していません")?;
                }
                parent = layers[p].parent;
            }
            for a in &l.references {
                check(
                    anchor_ids.get(a).copied() != Some(i as i32),
                    "自身のレイヤーのAnchorを参照しています",
                )?;
            }
        }
        validate_fields(&r.fields)?;
        Ok(Self {
            version,
            id,
            width,
            height,
            tile_size: ts,
            layers: count as usize,
            fields: r.fields,
        })
    }
}
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
    prefix: String,
    fields: Vec<NativeField>,
}
impl Reader<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self
            .at
            .checked_add(n)
            .ok_or_else(|| Error("長さが過大です".into()))?;
        let s = self.bytes.get(self.at..end).ok_or_else(|| {
            Error(format!(
                "正本が途中で切れています: {} (位置{})",
                self.prefix, self.at
            ))
        })?;
        self.at = end;
        Ok(s)
    }
    fn add(&mut self, n: &str, value: NativeValue) {
        self.fields.push(NativeField {
            path: if self.prefix.is_empty() {
                n.into()
            } else {
                format!("{}.{n}", self.prefix)
            },
            value,
        });
    }
    fn block<T>(&mut self, n: &str, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let old = self.prefix.clone();
        self.prefix = if old.is_empty() {
            n.into()
        } else {
            format!("{old}.{n}")
        };
        let result = f(self);
        self.prefix = old;
        result
    }
    fn int(&mut self, n: &str, min: i32, max: i32) -> Result<i32> {
        let v = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
        check(
            (min..=max).contains(&v),
            format!(
                "{}.{} の値 {v} は未対応または範囲外です ({min}..{max})",
                self.prefix, n
            ),
        )?;
        self.add(n, NativeValue::Int(v));
        Ok(v)
    }
    fn byte(&mut self, n: &str) -> Result<u8> {
        let v = self.take(1)?[0];
        self.add(n, NativeValue::Byte(v));
        Ok(v)
    }
    fn boolean(&mut self, n: &str) -> Result<bool> {
        let v = self.take(1)?[0];
        check(v <= 1, format!("{}.{} の真偽値が不正です", self.prefix, n))?;
        self.add(n, NativeValue::Bool(v != 0));
        Ok(v != 0)
    }
    fn float(&mut self, n: &str, min: f64, max: f64) -> Result<f64> {
        let v = f64::from_le_bytes(self.take(8)?.try_into().unwrap());
        check(
            v.is_finite() && v >= min && v <= max,
            format!("{}.{} の数値が有限でないか範囲外です", self.prefix, n),
        )?;
        self.add(n, NativeValue::Float(v));
        Ok(v)
    }
    fn unit(&mut self, n: &str) -> Result<f64> {
        self.float(n, 0., 1.)
    }
    fn id(&mut self, n: &str, empty: bool) -> Result<[u8; 16]> {
        let v: [u8; 16] = self.take(16)?.try_into().unwrap();
        check(empty || v != [0; 16], "IDが空です")?;
        self.add(n, NativeValue::Guid(v));
        Ok(v)
    }
    fn blob(&mut self, n: &str, len: usize) -> Result<Arc<[u8]>> {
        let v: Arc<[u8]> = Arc::from(self.take(len)?);
        self.add(n, NativeValue::Bytes(v.clone()));
        Ok(v)
    }
    fn string(&mut self, n: &str) -> Result<String> {
        let len = i32::from_le_bytes(self.take(4)?.try_into().unwrap());
        check((0..=4096).contains(&len), "文字列の長さが不正です")?;
        let s = std::str::from_utf8(self.take(len as usize)?)
            .map_err(|_| Error("文字列がUTF-8ではありません".into()))?
            .to_string();
        self.add(n, NativeValue::Text(s.clone()));
        Ok(s)
    }
}
struct Layer {
    id: [u8; 16],
    parent: [u8; 16],
    kind: i32,
    anchors: Vec<[u8; 16]>,
    references: Vec<[u8; 16]>,
}
/// 版 22 のユーザーチャンネルの一覧: 数（1〜58。0 の一覧は書かない）、番号の昇順に番号（6〜63）・名前（1〜128 文字、制御文字なし、
/// 標準の名前とも重ならない）・種類・色空間・既定の RGBA。
fn user_channels(r: &mut Reader<'_>) -> Result<UserChannels> {
    let n = r.int("user_channel_count", 1, 64 - STANDARD_CHANNELS)?;
    let mut names: HashSet<String> = [
        "Color",
        "Roughness",
        "Metallic",
        "Height",
        "Normal",
        "Emission",
    ]
    .into_iter()
    .map(String::from)
    .collect();
    let mut user = UserChannels::new();
    let mut previous = STANDARD_CHANNELS - 1;
    for i in 0..n {
        r.block(&format!("user_channels[{i}]"), |r| {
            let c = r.int("channel", STANDARD_CHANNELS, 63)?;
            check(c > previous, "ユーザーチャンネルの番号の並びが不正です")?;
            previous = c;
            let name = r.string("name")?;
            let chars = name.chars().count();
            check(
                (1..=128).contains(&chars) && !name.chars().any(char::is_control),
                "ユーザーチャンネルの名前が不正です",
            )?;
            check(names.insert(name), "チャンネルの名前が重複しています")?;
            let kind = r.int("kind", 0, 2)?;
            r.int("color_space", 0, 1)?;
            r.blob("default", 4)?;
            user.insert(c, kind);
            Ok(())
        })?;
    }
    Ok(user)
}
fn layer(
    r: &mut Reader<'_>,
    v: i32,
    w: i32,
    h: i32,
    ts: i32,
    user: &UserChannels,
) -> Result<Layer> {
    // 版 22 でユーザーチャンネルを置けるのは、チャンネルごとの合成・塗りつぶしの値・調整の対象・ラスターのチャンネル
    let channel_total = STANDARD_CHANNELS + user.len() as i32;
    let id = r.id("id", false)?;
    r.string("name")?;
    r.boolean("visible")?;
    r.unit("opacity")?;
    let blend = r.int("blend", 0, 26)?;
    let mut flags = 0;
    let mut blends = Vec::new();
    if v >= 12 {
        flags = r.byte("attributes")?;
        let known = 3
            | if v >= 14 { 4 } else { 0 }
            | if v >= 16 { 8 } else { 0 }
            | if v >= 20 { 16 } else { 0 }
            | if v >= 21 { 32 } else { 0 };
        check(flags & !known == 0, "未知のレイヤー属性ビットです")?;
        if flags & 2 != 0 {
            r.int("locks", 1, 15)?;
        }
        if flags & 4 != 0 {
            let n = r.byte("channel_blend_count")?;
            check(
                (1..=channel_total).contains(&(n as i32)),
                "チャンネル合成数が不正です",
            )?;
            let mut seen = HashSet::new();
            for i in 0..n {
                r.block(&format!("channel_blends[{i}]"), |r| {
                    layer_channel(r, &mut seen, user)?;
                    let p = r.byte("parts")?;
                    check((1..=3).contains(&p), "未知の合成属性です")?;
                    if p & 1 != 0 {
                        blends.push(r.int("mode", 0, 26)?);
                    }
                    if p & 2 != 0 {
                        r.unit("opacity")?;
                    }
                    Ok(())
                })?;
            }
        }
    } else if v >= 5 {
        r.boolean("clipping")?;
    }
    let kind = if v >= 3 {
        r.int(
            "kind",
            0,
            if v >= 6 {
                3
            } else if v >= 4 {
                2
            } else {
                1
            },
        )?
    } else {
        0
    };
    check(
        kind == 3 || blend != 26 && !blends.contains(&26),
        "通過合成はグループだけに使えます",
    )?;
    let parent = if v >= 6 {
        r.id("parent", true)?
    } else {
        [0; 16]
    };
    let mut fills = HashSet::new();
    let mut images = HashSet::new();
    let mut references = Vec::new();
    if v >= 3 {
        let n = r.int("fill_count", 0, if kind == 1 { channel_total } else { 0 })?;
        for i in 0..n {
            r.block(&format!("fills[{i}]"), |r| {
                layer_channel(r, &mut fills, user)?;
                r.boolean("enabled")?;
                r.blob("rgba", 4)?;
                Ok(())
            })?;
        }
    }
    if flags & 8 != 0 {
        check(kind == 1, "塗りつぶし以外に画像投影があります")?;
        let n = r.int("image_count", 0, 6)?;
        for i in 0..n {
            r.block(&format!("images[{i}]"), |r| {
                let c = unique_channel(r, &mut images)?;
                check(fills.contains(&c), "画像に対応する塗りつぶし値がありません")?;
                r.id("resource_id", false)?;
                Ok(())
            })?;
        }
        let default = r.block("projection", |r| projection(r, v))?;
        check(n > 0 || !default, "空の画像投影ブロックです")?;
    }
    if flags & 32 != 0 {
        check(kind == 1, "塗りつぶし以外にグラデーションがあります")?;
        let n = r.int("gradient_count", 1, 6)?;
        let mut seen = HashSet::new();
        for i in 0..n {
            r.block(&format!("gradients[{i}]"), |r| {
                let c = unique_channel(r, &mut seen)?;
                check(
                    c != 4 && fills.contains(&c) && !images.contains(&c),
                    "グラデーションのチャンネル・塗りつぶし元が不正です",
                )?;
                check(
                    generator(r, v, &mut references)? == 5,
                    "塗りつぶしグラデーションは形状Generatorが必要です",
                )?;
                Ok(())
            })?;
        }
    }
    if kind == 2 {
        r.block("adjustment", |r| {
            let t = r.int("type", 0, 2)?;
            r.int("algorithm", 1, 1)?;
            let mut p = Vec::new();
            for k in [
                "input_black",
                "input_white",
                "gamma",
                "output_black",
                "output_white",
                "hue",
                "saturation",
                "lightness",
            ] {
                p.push(r.float(k, -f64::MAX, f64::MAX)?);
            }
            if t == 1 {
                levels(&p[..5])?;
            }
            if t == 2 {
                check(
                    (-180. ..=180.).contains(&p[5])
                        && (-1. ..=1.).contains(&p[6])
                        && (-1. ..=1.).contains(&p[7]),
                    "色相・彩度・明度が範囲外です",
                )?;
            }
            let n = r.int("channel_count", 0, channel_total)?;
            let mut seen = HashSet::new();
            for i in 0..n {
                r.block(&format!("channels[{i}]"), |r| {
                    let c = layer_channel(r, &mut seen, user)?;
                    check(
                        t != 2 || c == 0 || c == 5 || user.get(&c) == Some(&0),
                        "調整対象のチャンネルが不正です",
                    )
                })?;
            }
            Ok(())
        })?;
    }
    let n = r.int(
        "channel_count",
        0,
        if kind == 0 { channel_total } else { 0 },
    )?;
    let mut channels = HashSet::new();
    let mut enabled = HashSet::new();
    for i in 0..n {
        r.block(&format!("channels[{i}]"), |r| {
            let c = layer_channel(r, &mut channels, user)?;
            if r.boolean("enabled")? {
                enabled.insert(c);
            }
            tiles(r, w, h, ts, false)
        })?;
    }
    let mask = v >= 2 && r.boolean("has_mask")?;
    if mask {
        r.block("mask", |r| {
            r.boolean("enabled")?;
            r.boolean("inverted")?;
            r.unit("density")?;
            tiles(r, w, h, ts, true)
        })?;
    }
    let surface = v >= 8 && r.boolean("has_surface_path")?;
    if surface {
        check(kind == 0, "パスはラスター層に限ります")?;
        r.block("surface_path", |r| path(r, v, true, &channels, &enabled))?;
    }
    if v >= 9 && r.boolean("has_filters")? {
        r.block("filters", |r| filters(r, v, true, &mut references))?;
        if mask {
            r.block("mask.filters", |r| filters(r, v, false, &mut references))?;
        }
    }
    if v >= 10 && r.boolean("has_canvas_path")? {
        check(!surface && kind == 0, "パスの種類またはレイヤーが不正です")?;
        r.block("canvas_path", |r| path(r, v, false, &channels, &enabled))?;
    }
    let mut anchors = Vec::new();
    if flags & 16 != 0 {
        let f = r.byte("anchor_flags")?;
        check(
            (1..=3).contains(&f) && (f & 2 == 0 || mask),
            "Anchorの属性またはマスクが不正です",
        )?;
        for (bit, name) in [(1, "anchor"), (2, "mask.anchor")] {
            if f & bit != 0 {
                r.block(name, |r| {
                    anchors.push(r.id("id", false)?);
                    let s = r.string("name")?;
                    check(
                        !s.trim().is_empty() && s.encode_utf16().count() <= 128,
                        "Anchorの名前が不正です",
                    )
                })?;
            }
        }
    }
    Ok(Layer {
        id,
        parent,
        kind,
        anchors,
        references,
    })
}
/// 標準のチャンネルだけの項目（塗りつぶしの画像・グラデーション、フィルター、パスのマテリアル）。
fn unique_channel(r: &mut Reader<'_>, seen: &mut HashSet<i32>) -> Result<i32> {
    let c = r.int("channel", 0, STANDARD_CHANNELS - 1)?;
    check(seen.insert(c), "チャンネルが重複しています")?;
    Ok(c)
}
/// ユーザーチャンネルも置ける項目。版 21 までは標準だけ、版 22 は一覧にある番号だけ。
fn layer_channel(r: &mut Reader<'_>, seen: &mut HashSet<i32>, user: &UserChannels) -> Result<i32> {
    let max = if user.is_empty() {
        STANDARD_CHANNELS - 1
    } else {
        63
    };
    let c = r.int("channel", 0, max)?;
    check(
        c < STANDARD_CHANNELS || user.contains_key(&c),
        format!(
            "{}.channel {c} は文書のチャンネルの一覧にありません",
            r.prefix
        ),
    )?;
    check(seen.insert(c), "チャンネルが重複しています")?;
    Ok(c)
}
fn tiles(r: &mut Reader<'_>, w: i32, h: i32, ts: i32, mask: bool) -> Result<()> {
    let cols = (w + ts - 1) / ts;
    let rows = (h + ts - 1) / ts;
    let n = r.int("tile_count", 0, cols * rows)?;
    let mut seen = HashSet::new();
    for i in 0..n {
        r.block(&format!("tiles[{i}]"), |r| {
            let x = r.int("x", 0, cols - 1)?;
            let y = r.int("y", 0, rows - 1)?;
            check(seen.insert((x, y)), "タイルが重複しています")?;
            let len = r.int("length", ts * ts * 4, ts * ts * 4)?;
            let b = r.blob("rgba", len as usize)?;
            check(
                !mask || b.as_chunks::<4>().0.iter().all(|p| p[..3] == [0, 0, 0]),
                "マスクのRGBは0でなければなりません",
            )
        })?;
    }
    Ok(())
}
fn levels(p: &[f64]) -> Result<()> {
    check(
        p[0] >= 0.
            && p[1] <= 1.
            && p[1] - p[0] >= 1. / 255.
            && (0.1..=9.99).contains(&p[2])
            && (0. ..=1.).contains(&p[3])
            && (0. ..=1.).contains(&p[4]),
        "レベル補正が範囲外です",
    )
}
fn volume(r: &mut Reader<'_>, falloff: bool) -> Result<Vec<f64>> {
    let mut p = Vec::new();
    for (names, min, max) in [
        (["center_x", "center_y", "center_z"], -1e6, 1e6),
        (["rotation_x", "rotation_y", "rotation_z"], -360., 360.),
        (["size_x", "size_y", "size_z"], 1e-6, 1e6),
    ] {
        for n in names {
            p.push(r.float(n, min, max)?);
        }
    }
    if falloff {
        p.push(r.unit("falloff")?);
    }
    Ok(p)
}
fn projection(r: &mut Reader<'_>, v: i32) -> Result<bool> {
    r.int("algorithm", 1, 1)?;
    let mode = r.int("mode", 0, if v >= 17 { 5 } else { 4 })?;
    let wrap = r.int("wrap", 0, if v >= 17 { 2 } else { 1 })?;
    let u = r.float("tile_u", 1e-3, 1e4)?;
    let vv = r.float("tile_v", 1e-3, 1e4)?;
    let ou = r.float("offset_u", -1e4, 1e4)?;
    let ov = r.float("offset_v", -1e4, 1e4)?;
    let rot = r.float("rotation", -360., 360.)?;
    let blend = r.unit("blend_width")?;
    let p = r.block("placement", |r| volume(r, false))?;
    if mode == 5 {
        r.unit("depth_hardness")?;
        r.float("backface_angle", 0., 180.)?;
        r.unit("backface_hardness")?;
    }
    Ok(mode == 0
        && wrap == 0
        && u == 1.
        && vv == 1.
        && ou == 0.
        && ov == 0.
        && rot == 0.
        && blend == 0.3
        && p == [0., 0., 0., 0., 0., 0., 1., 1., 1.])
}
fn generator(r: &mut Reader<'_>, v: i32, refs: &mut Vec<[u8; 16]>) -> Result<i32> {
    let t = r.int(
        "type",
        0,
        if v >= 20 {
            7
        } else if v >= 15 {
            6
        } else if v >= 13 {
            5
        } else {
            4
        },
    )?;
    let algorithm = r.int("algorithm", 1, if t == 5 && v >= 21 { 2 } else { 1 })?;
    let low = r.unit("low")?;
    let high = r.unit("high")?;
    check(high - low >= 0.001, "Generatorのレベル幅が不足しています")?;
    r.unit("softness")?;
    r.boolean("invert")?;
    r.unit("noise_amount")?;
    r.float("noise_scale", 0.001, 1.)?;
    r.int("noise_seed", i32::MIN, i32::MAX)?;
    r.int("noise_space", 0, 1)?;
    r.int("blend", 0, 6)?;
    let balance = r.unit("balance")?;
    let axis = r.int("axis", 0, 2)?;
    let x = r.float("direction_x", -1e6, 1e6)?;
    let y = r.float("direction_y", -1e6, 1e6)?;
    let z = r.float("direction_z", -1e6, 1e6)?;
    let bent = r.boolean("bent_normal")?;
    check(
        (t == 1 || balance == 0.5) && (t == 2 || axis == 1),
        "Generator固有でない属性が変更されています",
    )?;
    check(
        if t == 4 {
            x * x + y * y + z * z >= 1e-12
        } else {
            x == 0. && y == 1. && z == 0. && !bent
        },
        "Generatorの方向が不正です",
    )?;
    let n = r.int("pin_count", 0, 8)?;
    let mut seen = HashSet::new();
    let candidates: &[i32] = match t {
        0 => &[3, 1],
        1 => &[2, 3, 1],
        2 | 5 | 7 => &[1],
        3 => &[4, 1],
        6 => &[7, 1],
        _ => &[0, 8, 1],
    };
    for i in 0..n {
        r.block(&format!("pins[{i}]"), |r| {
            let k = r.int("kind", 0, 9)?;
            check(
                seen.insert(k) && candidates.contains(&k),
                "Generatorが使わないか重複したメッシュマップです",
            )?;
            check(
                is_hash(&r.string("key")?),
                "メッシュマップの条件キーが不正です",
            )
        })?;
    }
    if t == 5 {
        r.block("volume", |r| {
            r.int("shape", 0, 2)?;
            volume(r, true)?;
            Ok(())
        })?;
        if algorithm == 2 {
            r.block("ramp", ramp)?;
        }
    }
    if t == 6 {
        r.int("tolerance", 0, 255)?;
        let n = r.int("color_count", 0, 32)?;
        let mut seen = HashSet::new();
        for i in 0..n {
            check(
                seen.insert(r.int(&format!("colors[{i}]"), 0, 0xffffff)?),
                "GeneratorのID色が重複しています",
            )?;
        }
    }
    if t == 7 {
        refs.push(r.id("anchor_id", true)?);
        check(
            r.int("anchor_channel", 0, 5)? != 4,
            "AnchorはNormalを読めません",
        )?;
        r.int("anchor_read", 0, 1)?;
    }
    Ok(t)
}
fn ramp(r: &mut Reader<'_>) -> Result<()> {
    for (kind, max) in [("colors", 32), ("opacities", 32), ("curve", 16)] {
        let n = r.int(&format!("{kind}_count"), 2, max)?;
        let mut previous = -1.;
        for i in 0..n {
            r.block(&format!("{kind}[{i}]"), |r| {
                let p = r.unit(if kind == "curve" { "x" } else { "position" })?;
                let gap = if kind == "curve" { 0.02 - 1e-6 } else { 0.0001 };
                check(
                    i == 0 || p - previous >= gap,
                    "ランプの点が昇順でないか近すぎます",
                )?;
                previous = p;
                if kind == "curve" {
                    check(
                        (i != 0 || p == 0.) && (i != n - 1 || p == 1.),
                        "カーブは0から1まで必要です",
                    )?;
                    r.unit("y")?;
                } else {
                    if kind == "colors" {
                        r.blob("rgb", 3)?;
                    } else {
                        r.unit("opacity")?;
                    }
                    r.float("midpoint", 0.01, 0.99)?;
                }
                Ok(())
            })?;
        }
    }
    Ok(())
}
fn filters(r: &mut Reader<'_>, v: i32, content: bool, refs: &mut Vec<[u8; 16]>) -> Result<()> {
    let n = r.int("count", 0, 32)?;
    let mut ids = HashSet::new();
    let mut halos = [0; 6];
    for i in 0..n {
        r.block(&format!("items[{i}]"), |r| {
            check(
                ids.insert(r.id("id", false)?),
                "フィルターIDが重複しています",
            )?;
            let t = r.int("type", 0, if v >= 11 { 6 } else { 5 })?;
            r.int("algorithm", 1, 1)?;
            let active = r.boolean("enabled")?;
            let strength = r.unit("strength")?;
            let mut channels = HashSet::new();
            if content {
                let n = r.int("channel_count", 1, 6)?;
                for i in 0..n {
                    r.block(&format!("channels[{i}]"), |r| {
                        unique_channel(r, &mut channels)
                    })?;
                }
            }
            let radius = r.int("radius", 0, 256)?;
            let amount = r.float("amount", 0., 5.)?;
            let threshold = r.int("threshold", 0, 255)?;
            let seed = r.int("seed", i32::MIN, i32::MAX)?;
            let mono = r.boolean("monochrome")?;
            let mut p = Vec::new();
            for name in [
                "input_black",
                "input_white",
                "gamma",
                "output_black",
                "output_white",
            ] {
                p.push(r.float(name, -f64::MAX, f64::MAX)?);
            }
            check(
                match t {
                    0 => (1..=256).contains(&radius),
                    1 => (1..=64).contains(&radius),
                    _ => radius == 0,
                },
                "フィルターの半径が不正です",
            )?;
            check(
                (t == 1 || t == 2 && amount <= 1. || amount == 0.)
                    && (t == 1 || threshold == 0)
                    && (t == 2 || seed == 0 && !mono),
                "フィルターのパラメーターが不正です",
            )?;
            if t == 3 {
                levels(&p)?;
            } else {
                check(
                    p == [0., 1., 1., 0., 1.],
                    "未使用のレベル補正値が変更されています",
                )?;
            }
            if channels.contains(&4) {
                check(t == 0, "Normalに適用できないフィルターです")?;
            }
            if t == 2 && !mono {
                check(
                    content && !channels.iter().any(|c| [1, 2, 3].contains(c)),
                    "スカラーチャンネルにカラーNoiseを適用できません",
                )?;
            }
            if t == 6 {
                r.block("generator", |r| generator(r, v, refs))?;
            }
            if active && strength > 0. {
                for (channel, halo) in halos.iter_mut().enumerate() {
                    if !content || channels.contains(&(channel as i32)) {
                        *halo += radius;
                        check(
                            *halo <= 512,
                            "フィルタースタックの到達半径が512を超えています",
                        )?;
                    }
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}
fn path(
    r: &mut Reader<'_>,
    v: i32,
    surface: bool,
    channels: &HashSet<i32>,
    enabled: &HashSet<i32>,
) -> Result<()> {
    r.int("algorithm", 1, 1)?;
    r.id("id", true)?;
    let channel = r.int("channel", 0, 5)?;
    if surface {
        let s = r.string("model_fingerprint")?;
        check(
            !s.is_empty() && s.encode_utf16().count() <= 128,
            "パスのモデル指紋が不正です",
        )?;
    }
    r.block("brush", |r| {
        let radius = r.float("radius", 0., if surface { 1e6 } else { 4096. })?;
        check(radius > 0., "パスの半径は正数です")?;
        r.unit("hardness")?;
        r.float("spacing", 0.01, 4.)?;
        r.unit("opacity")?;
        r.unit("flow")?;
        r.blob("rgba", 4)?;
        for k in [
            "erase",
            "pressure_size",
            "pressure_opacity",
            "pressure_flow",
        ] {
            r.boolean(k)?;
        }
        Ok(())
    })?;
    let n = r.int("point_count", 0, 4096)?;
    for i in 0..n {
        r.block(&format!("points[{i}]"), |r| {
            if surface {
                r.int("triangle", 0, i32::MAX)?;
                let u = r.float("u", -1e-9, 1. + 1e-9)?;
                let vv = r.float("v", -1e-9, 1. + 1e-9)?;
                check(u + vv <= 1. + 1e-9, "パスの点が三角形の外です")?;
            } else {
                r.float("x", -1e6, 1e6)?;
                r.float("y", -1e6, 1e6)?;
            }
            r.unit("pressure")?;
            Ok(())
        })?;
    }
    let count = if v >= 18 {
        r.byte("material_count")?
    } else {
        0
    };
    check(count <= 6, "パスのマテリアル数が不正です")?;
    if count == 0 {
        check(
            enabled.contains(&channel),
            "パスのチャンネルが有効ではありません",
        )?;
    }
    let mut seen = HashSet::new();
    for i in 0..count {
        r.block(&format!("material[{i}]"), |r| {
            let c = unique_channel(r, &mut seen)?;
            check(
                channels.contains(&c),
                "パスのマテリアルのチャンネルがありません",
            )?;
            r.blob("rgba", 4)?;
            Ok(())
        })?;
    }
    Ok(())
}

fn validate_fields(fields: &[NativeField]) -> Result<()> {
    let map: HashMap<&str, &NativeValue> =
        fields.iter().map(|f| (f.path.as_str(), &f.value)).collect();
    let mut filter_ids = HashSet::new();
    for field in fields {
        if field.path.contains(".filters.items[") && field.path.ends_with(".id") {
            if let NativeValue::Guid(id) = field.value {
                check(
                    filter_ids.insert(id),
                    "文書内のフィルターIDが重複しています",
                )?;
            }
        }
        if field.path.contains(".gradients[") && field.path.ends_with(".type") {
            let prefix = field.path.strip_suffix(".type").unwrap();
            check(
                map.get(format!("{prefix}.algorithm").as_str()) == Some(&&NativeValue::Int(2))
                    && map.get(format!("{prefix}.blend").as_str()) == Some(&&NativeValue::Int(1)),
                "塗りつぶしグラデーションはランプ付き・Replaceが必要です",
            )?;
        }
        if field.path.ends_with(".filters.count") && !field.path.contains(".mask.") {
            let layer = field.path.strip_suffix(".filters.count").unwrap();
            let kind = map.get(format!("{layer}.kind").as_str());
            check(
                field.value == NativeValue::Int(0)
                    || !matches!(kind, Some(NativeValue::Int(2 | 3))),
                "調整・グループには内容フィルターを設定できません",
            )?;
        }
    }
    Ok(())
}
