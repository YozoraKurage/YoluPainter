use crate::{check, check_budget, guid, is_hash, Error, Result, MAX_ENTRY_BYTES};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

/// Unity 版（0.2.0 の `DocumentBinary`）が書き、読める正本の一番新しい版。
pub const UNITY_NATIVE_VERSION: i32 = 21;
/// 文書のユーザーチャンネル（core の 6〜63）の一覧を足した版。ユーザーチャンネルのある文書だけがこの版になり、
/// Unity 版の読み手は「Unsupported archive version」で断る（形式と決めは README の「ユーザーチャンネル（正本の版 22）」）。
pub const USER_CHANNELS_VERSION: i32 = 22;
/// Rust 版だけの Generator の種類（ノイズ 64・グランジ 65）を足した版。版 22 の中身（ユーザーチャンネルの一覧。この版では 0 個も書く）に、
/// Generator の種類 64・65 とその欄が加わる。これを使う文書だけがこの版になり、Unity 版の読み手は「Unsupported archive version」で断る
/// （形式と決めは README の「手続き型の Generator（正本の版 23）」）。
pub const PROCEDURAL_VERSION: i32 = 23;
/// Rust 版だけの色調補正（調整の層とフィルターの段の種類 64〜69: グラデーションマップ・トーンカーブ・カラーバランス・明るさ/コントラスト・
/// 2 値化・ポスタリゼーション）を足した版。版 23 の中身に、調整・フィルターの種類 64〜69 とその欄（`color_adjust` の並び）が加わる。これを使う
/// 文書だけがこの版になり、Unity 版の読み手は「Unsupported archive version」で断る（形式と決めは README の「色調補正（正本の版 24）」）。
pub const ADJUST_VERSION: i32 = 24;
/// グラデーションマップの混色（混色モード・輝度の補正）と区間ごとの混合率曲線を足した版。版 24 の中身に、種類 64（グラデーションマップ）の欄の
/// ランプのあとへ混色の欄が加わる。これらを使うグラデーションマップのある文書だけがこの版になり、Unity 版の読み手は「Unsupported archive
/// version」で断る（形式と決めは README の「グラデーションマップの混色（正本の版 25）」）。
pub const MIXING_VERSION: i32 = 25;
/// この読み手が読める一番新しい版。
pub const MAX_NATIVE_VERSION: i32 = MIXING_VERSION;
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
            .ok_or_else(|| Error::InvalidData(format!("正本の項目がありません: {path}")))?;
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
        check_budget(
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
            user_channels(&mut r, version)?
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
                    .ok_or_else(|| Error::InvalidData("親グループがありません".into()))?;
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
            .ok_or_else(|| Error::Budget("長さが過大です".into()))?;
        let s = self.bytes.get(self.at..end).ok_or_else(|| {
            Error::InvalidData(format!(
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
            .map_err(|_| Error::InvalidData("文字列がUTF-8ではありません".into()))?
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
/// 版 22 のユーザーチャンネルの一覧: 数（版 22 は 1〜58で 0 の一覧は書かない。版 23 は 0〜58）、番号の昇順に番号（6〜63）・名前
/// （1〜128 文字、制御文字なし、標準の名前とも重ならない）・種類・色空間・既定の RGBA。
fn user_channels(r: &mut Reader<'_>, version: i32) -> Result<UserChannels> {
    let n = r.int(
        "user_channel_count",
        i32::from(version < PROCEDURAL_VERSION),
        64 - STANDARD_CHANNELS,
    )?;
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
                    "塗りつぶしグラデーションは形状ジェネレーターが必要です",
                )?;
                Ok(())
            })?;
        }
    }
    if kind == 2 {
        r.block("adjustment", |r| {
            let t = r.int(
                "type",
                0,
                if v >= ADJUST_VERSION {
                    ADJUST_KIND_MAX
                } else {
                    2
                },
            )?;
            // 3〜63 は Unity 版の将来のために空けてある（Rust 版は使わない）
            check(!(3..ADJUST_KIND_MIN).contains(&t), "未知の調整の種類です")?;
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
            if t >= ADJUST_KIND_MIN {
                // 64 からの種類は 8 つの値を使わない（既定のまま）。値は種類ごとの欄に続く
                check(
                    p == [0., 1., 1., 0., 1., 0., 0., 0.],
                    "未使用の調整の値が変更されています",
                )?;
                r.block("detail", |r| color_adjust(r, v, t))?;
            }
            let n = r.int("channel_count", 0, channel_total)?;
            let mut seen = HashSet::new();
            for i in 0..n {
                r.block(&format!("channels[{i}]"), |r| {
                    let c = layer_channel(r, &mut seen, user)?;
                    // 色だけの種類（色相/彩度・グラデーションマップ・カラーバランス）は色のチャンネルだけ。
                    // トーンカーブ・明るさ/コントラスト・2 値化・ポスタリゼーションは法線に当てない
                    let colour_only = matches!(t, 2 | 64 | 66);
                    let not_normal = matches!(t, 65 | 67..=69);
                    check(
                        (!colour_only || c == 0 || c == 5 || user.get(&c) == Some(&0))
                            && (!not_normal || (c != 4 && user.get(&c) != Some(&2))),
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
        if v >= PROCEDURAL_VERSION {
            PROCEDURAL_KIND_MAX
        } else if v >= 20 {
            7
        } else if v >= 15 {
            6
        } else if v >= 13 {
            5
        } else {
            4
        },
    )?;
    // 8〜63 は Unity 版の将来のために空けてある（Rust 版は使わない）
    check(
        !(8..PROCEDURAL_KIND_MIN).contains(&t),
        "未知のジェネレーターの種類です",
    )?;
    let algorithm = r.int("algorithm", 1, if t == 5 && v >= 21 { 2 } else { 1 })?;
    let low = r.unit("low")?;
    let high = r.unit("high")?;
    check(high - low >= 0.001, "ジェネレーターのレベル幅が不足しています")?;
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
        "ジェネレーター固有でない属性が変更されています",
    )?;
    check(
        if t == 4 {
            x * x + y * y + z * z >= 1e-12
        } else {
            x == 0. && y == 1. && z == 0. && !bent
        },
        "ジェネレーターの方向が不正です",
    )?;
    let n = r.int("pin_count", 0, 8)?;
    let mut seen = HashSet::new();
    let candidates: &[i32] = match t {
        0 => &[3, 1],
        1 => &[2, 3, 1],
        2 | 5 | 7 => &[1],
        3 => &[4, 1],
        6 => &[7, 1],
        PROCEDURAL_KIND_MIN.. => &[1, 0],
        _ => &[0, 8, 1],
    };
    for i in 0..n {
        r.block(&format!("pins[{i}]"), |r| {
            let k = r.int("kind", 0, 9)?;
            check(
                seen.insert(k) && candidates.contains(&k),
                "ジェネレーターが使わないか重複したメッシュマップです",
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
                "ジェネレーターのID色が重複しています",
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
    if t >= PROCEDURAL_KIND_MIN {
        r.block("procedural", |r| procedural(r, t))?;
    }
    Ok(t)
}
/// Rust 版だけの Generator の種類の番号（ノイズ 64・グランジ 65）。
const PROCEDURAL_KIND_MIN: i32 = 64;
const PROCEDURAL_KIND_MAX: i32 = 65;
/// ノイズ・グランジの欄: 空間・模様の大きさ・シード・回転・にじみ・トライプラナーの幅に、ノイズ（64）は基底・セルの出力・重ね方・
/// オクターブ・ラクナリティ・ゲイン、グランジ（65）はプリセット。
fn procedural(r: &mut Reader<'_>, t: i32) -> Result<()> {
    r.int("space", 0, 2)?;
    r.float("scale", 0.001, 1.)?;
    r.int("seed", i32::MIN, i32::MAX)?;
    for axis in ["rotation_x", "rotation_y", "rotation_z"] {
        r.float(axis, -360., 360.)?;
    }
    r.unit("bleed")?;
    r.unit("blend_width")?;
    if t == PROCEDURAL_KIND_MIN {
        let basis = r.int("basis", 0, 2)?;
        let cell = r.int("cell_output", 0, 2)?;
        check(basis == 2 || cell == 0, "セルの出力はWorley専用です")?;
        r.int("fractal", 0, 2)?;
        r.int("octaves", 1, 8)?;
        r.float("lacunarity", 1., 4.)?;
        r.unit("gain")?;
    } else {
        r.int("preset", 0, 10)?;
    }
    Ok(())
}
/// Rust 版だけの調整・フィルターの種類の番号（グラデーションマップ 64〜ポスタリゼーション 69）。
const ADJUST_KIND_MIN: i32 = 64;
const ADJUST_KIND_MAX: i32 = 69;
/// 64 からの調整・フィルターの種類ごとの欄（調整の層は `detail`、フィルターの段は `adjust` のブロックの中）。
/// 64: 逆向き・ランプ。65: 合成・R・G・B の 4 本のカーブ。66: 範囲ごとの 3 本のスライダーと輝度を保つ。
/// 67: 明るさ・コントラスト。68: しきい値。69: 階調。
fn color_adjust(r: &mut Reader<'_>, v: i32, t: i32) -> Result<()> {
    match t {
        64 => {
            r.boolean("reverse")?;
            let colors = r.block("ramp", |r| {
                let colors = ramp(r)?;
                Ok(colors)
            })?;
            if v >= MIXING_VERSION {
                gradient_mixing(r, colors)?;
            }
        }
        65 => {
            for name in ["composite", "red", "green", "blue"] {
                curve_points(r, name)?;
            }
        }
        66 => {
            for range in ["shadows", "midtones", "highlights"] {
                for axis in ["cyan_red", "magenta_green", "yellow_blue"] {
                    r.float(&format!("{range}_{axis}"), -100., 100.)?;
                }
            }
            r.boolean("preserve_luminosity")?;
        }
        67 => {
            r.float("brightness", -150., 150.)?;
            r.float("contrast", -50., 100.)?;
        }
        68 => {
            r.int("level", 1, 255)?;
        }
        _ => {
            r.int("levels", 2, 255)?;
        }
    }
    Ok(())
}
/// 値のカーブの点（数 2〜16、x は 0 から 1 まで昇順で間隔 0.02 − 1e-6 以上、y は 0〜1）。欄の名前は `{name}_count`・`{name}[i].x`・`{name}[i].y`。
fn curve_points(r: &mut Reader<'_>, name: &str) -> Result<()> {
    let n = r.int(&format!("{name}_count"), 2, 16)?;
    let mut previous = -1.;
    for i in 0..n {
        r.block(&format!("{name}[{i}]"), |r| {
            let p = r.unit("x")?;
            check(
                i == 0 || p - previous >= 0.02 - 1e-6,
                "ランプの点が昇順でないか近すぎます",
            )?;
            previous = p;
            check(
                (i != 0 || p == 0.) && (i != n - 1 || p == 1.),
                "カーブは0から1まで必要です",
            )?;
            r.unit("y")?;
            Ok(())
        })?;
    }
    Ok(())
}
/// グラデーションマップの混色の欄（正本の版 25。ランプのあと）: 混色モード（0 通常・1 知覚的・2 リニア）、輝度の補正（0〜4。知覚的でなければ
/// 既定の 3）、区間の数（色の分岐点の数 − 1）と、区間ごとの `enabled` と、あれば混合率曲線 `curve`。
fn gradient_mixing(r: &mut Reader<'_>, colors: i32) -> Result<()> {
    let mode = r.int("mix", 0, 2)?;
    let correction = r.int("luminance", 0, 4)?;
    check(
        mode == 1 || correction == 3,
        "輝度の補正は知覚的な混色のときだけです",
    )?;
    let n = r.int("segment_count", 1, 31)?;
    check(
        n == colors - 1,
        "混合率曲線の区間の数が色の分岐点と合いません",
    )?;
    for i in 0..n {
        r.block(&format!("segments[{i}]"), |r| {
            if r.boolean("enabled")? {
                curve_points(r, "curve")?;
            }
            Ok(())
        })?;
    }
    Ok(())
}
/// ランプ（色・不透明度の分岐点と値のカーブ）。色の分岐点の数を返す。
fn ramp(r: &mut Reader<'_>) -> Result<i32> {
    let mut colors = 0;
    for (kind, max) in [("colors", 32), ("opacities", 32)] {
        let n = r.int(&format!("{kind}_count"), 2, max)?;
        if kind == "colors" {
            colors = n;
        }
        let mut previous = -1.;
        for i in 0..n {
            r.block(&format!("{kind}[{i}]"), |r| {
                let p = r.unit("position")?;
                check(
                    i == 0 || p - previous >= 0.0001,
                    "ランプの点が昇順でないか近すぎます",
                )?;
                previous = p;
                if kind == "colors" {
                    r.blob("rgb", 3)?;
                } else {
                    r.unit("opacity")?;
                }
                r.float("midpoint", 0.01, 0.99)?;
                Ok(())
            })?;
        }
    }
    curve_points(r, "curve")?;
    Ok(colors)
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
            let t = r.int(
                "type",
                0,
                if v >= ADJUST_VERSION {
                    ADJUST_KIND_MAX
                } else if v >= 11 {
                    6
                } else {
                    5
                },
            )?;
            // 7〜63 は Unity 版の将来のために空けてある（Rust 版は使わない）
            check(
                !(7..ADJUST_KIND_MIN).contains(&t),
                "未知のフィルターの種類です",
            )?;
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
            if t >= ADJUST_KIND_MIN {
                // グラデーションマップ・カラーバランスは色のチャンネルだけ（スカラーのチャンネルとマスクには置けない）
                if matches!(t, 64 | 66) {
                    check(
                        content && !channels.iter().any(|c| [1, 2, 3].contains(c)),
                        "スカラーチャンネルとマスクに色だけの調整を適用できません",
                    )?;
                }
                r.block("adjust", |r| color_adjust(r, v, t))?;
            }
            if active && strength > 0. {
                for (channel, halo) in halos.iter_mut().enumerate() {
                    if !content || channels.contains(&(channel as i32)) {
                        *halo += radius;
                        check_budget(
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
