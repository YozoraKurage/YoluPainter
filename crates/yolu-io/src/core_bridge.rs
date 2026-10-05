//! 正本（`NativeDocument`）と core の文書の行き来。意味は Unity 版の `DocumentBinary.Read` / `Write` と同じにする（C# が書いた正本は
//! core を通して書き戻すとバイト一致する）。範囲は M2 の層（層の種類・入れ子と通過・分離・ラスターマスク・クリッピング・層のロック（版 12）・
//! チャンネルごとの有効と合成（版 14）・Normal の出力の設定（版 7）・版 22 のユーザーチャンネル）と、効果（フィルターのスタックと Generator の段
//! （版 9・11・13・15）、Anchor（版 20）、塗りつぶしの画像と投影（版 16・17）、塗りつぶしのグラデーション（版 21））と、編集できる 2D・3D のパス
//! （版 8・10・18）。core に無い項目（手動の ID 色）は先に検査して断り、部分変換を返さない。
use crate::native::{
    ADJUST_VERSION, MIXING_VERSION, PROCEDURAL_VERSION, UNITY_NATIVE_VERSION,
    USER_CHANNELS_VERSION,
};
use crate::{
    check, check_budget, Error, NativeDocument, NativeValue as V, Result, Unwritable, MAX_ENTRY_BYTES,
};
use std::collections::HashMap;
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::fill_image::{Placement, Projection, ProjectionMode, Wrap};
use yolu_core::generator::{
    self, anchor, ColorStop, LuminanceCorrection, MapKind, MixMode, OpacityStop, Ramp,
};
use yolu_core::paths;
use yolu_core::{
    AdjustmentSettings, AdjustmentType, AnchorId, AnchorPlacement, BalanceRange, BlendMode,
    BrightnessContrast, BrushSettings, Channel, ChannelBlend, ChannelInfo, ChannelKind,
    ColorAdjust, ColorBalance, ColorSpace, Document, EffectSettings, FilterEffect, FilterId,
    FilterSpec, FilterTarget, GradientMap, HeightEdgeMode, ImageId, LayerId, LayerKind, LayerLocks,
    LayerPath, NormalSettings, NormalYDirection, Posterize, Rgba8, Threshold, TileCoord,
    ToneChannel, ToneCurves,
};

fn core_id(mut guid: [u8; 16]) -> u128 {
    guid[..4].reverse();
    guid[4..6].reverse();
    guid[6..8].reverse();
    u128::from_be_bytes(guid)
}
pub(crate) fn native_id(id: u128) -> [u8; 16] {
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
pub(crate) struct Fields<'a>(HashMap<&'a str, &'a V>);
impl<'a> Fields<'a> {
    fn new(doc: &'a NativeDocument) -> Self {
        Self::of(doc.fields())
    }
    pub(crate) fn of(fields: &'a [crate::NativeField]) -> Self {
        Self(fields.iter().map(|f| (f.path.as_str(), &f.value)).collect())
    }
    fn get(&self, p: &str) -> Result<&'a V> {
        self.0
            .get(p)
            .copied()
            .ok_or_else(|| Error::InvalidData(format!("正本の項目がありません: {p}")))
    }
    fn has(&self, p: &str) -> bool {
        self.0.contains_key(p)
    }
    fn wrong(p: &str) -> Error {
        Error::InvalidData(format!("正本の項目の型が違います: {p}"))
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
            .ok_or_else(|| Error::InvalidData(format!("{p} のチャンネル {c} は範囲外です")))
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
        | "locks"
        | "has_surface_path"
        | "has_filters"
        | "has_canvas_path"
        | "filters"
        | "surface_path"
        | "canvas_path"
        | "anchor_flags"
        | "anchor"
        | "image_count"
        | "images"
        | "projection"
        | "gradient_count"
        | "gradients" => None,
        "mask" => match parts.next().unwrap_or_default().split('[').next() {
            Some(
                "enabled" | "inverted" | "density" | "tile_count" | "tiles" | "filters" | "anchor",
            ) => None,
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
        _ => Some((path.into(), "core に無い項目")),
    }
}

impl NativeDocument {
    /// core へ渡すと失われる項目（層の機能ごとに 1 つ、`layers[2].filters（フィルター・Generator）` の形）。空なら変換の対象
    /// （タイルの余白・画素の予算は変換時に検査する）。非表示・無効の層やマスクの中の項目も省略せずに断る。
    pub fn core_issues(&self) -> Vec<String> {
        core_issues_of(self.fields())
    }

    /// 編集用の core の文書にする（C# の `DocumentBinary.Read` と同じ意味）。文書と層の ID、透明の画素の RGB を保ち、読み込みを
    /// Undo の履歴に残さない。元の `NativeDocument` は変えない。
    pub fn to_core(&self) -> Result<Document> {
        self.to_core_within(None)
    }

    /// `to_core` の画素の予算を指定できる形（None は core の既定、`DEFAULT_SOURCE_BUDGET_BYTES`）。超えたら、どの層のどのタイルで
    /// 断ったかを添えて `Error::Budget` で断る（壊れたファイルとは区別できる）。
    pub fn to_core_within(&self, source_budget: Option<u64>) -> Result<Document> {
        check(!self.is_skeleton(), "骨組みの正本は core にできません")?;
        let issues = self.core_issues();
        check(
            issues.is_empty(),
            format!("coreへの変換を拒否しました: {}", issues.join("、")),
        )?;
        let f = Fields::new(self);
        let mut load = CoreLoad::begin(
            &f,
            self.version(),
            (self.width(), self.height(), self.tile_size()),
            source_budget,
        )?;
        for i in 0..self.layer_count() {
            load.layer(&f, i)?;
        }
        load.finish(&f)
    }

    /// core の文書から正本を作る（C# の `DocumentBinary.Write` と同じ並び）。ユーザーチャンネルが無ければ Unity 版と同じ版 21、あれば
    /// 版 22。履歴は保存しない。正本の範囲外の寸法・タイル寸法・層の数・名前、進行中のストローク、まだ書けない手動の ID 色は断る。値の無い塗りつぶしのチャンネルと
    /// グループの有効の印は、合成に効かず C# の書き手も書かないので書かない。
    pub fn from_core(doc: &Document) -> Result<Self> {
        check_writable(doc)?;
        let mut sink = VecSink(Vec::new());
        write_document(&mut sink, doc, version_of(doc))?;
        Self::read(&sink.0)
    }
}

/// core へ渡すと失われる項目（`NativeDocument::core_issues` と、骨組みの項目から）。
pub(crate) fn core_issues_of(fields: &[crate::NativeField]) -> Vec<String> {
    let map: HashMap<&str, &V> = fields.iter().map(|f| (f.path.as_str(), &f.value)).collect();
    let mut issues = Vec::new();
    for f in fields {
        if let Some((key, why)) = unsupported(&f.path, &map) {
            let issue = format!("{key}（{why}）");
            if !issues.contains(&issue) {
                issues.push(issue);
            }
        }
    }
    issues
}

/// 正本を core の文書へ層ごとに入れる（頭 → 層 → 終わり）。メモリの正本も、流して読む正本も同じ道を通る。
pub(crate) struct CoreLoad {
    doc: Document,
    version: i32,
    ids: Vec<LayerId>,
    parents: Vec<[u8; 16]>,
    locks: Vec<LayerLocks>,
}
impl CoreLoad {
    /// 頭（寸法・Normal の設定・ユーザーチャンネル）から空の文書を作る。`f` は頭の項目を含む。
    pub(crate) fn begin(
        f: &Fields<'_>,
        version: i32,
        (width, height, tile_size): (i32, i32, i32),
        source_budget: Option<u64>,
    ) -> Result<Self> {
        let mut doc = Document::with_tile_size(width as u32, height as u32, tile_size as u32)?;
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
                    .map_err(|e| Error::from(e).in_context(format!("{p}をcoreにできません")))?;
            }
        }
        Ok(Self {
            doc,
            version,
            ids: Vec::new(),
            parents: Vec::new(),
            locks: Vec::new(),
        })
    }
    /// 層 `i` を足す（`f` はその層の項目を含む）。
    pub(crate) fn layer(&mut self, f: &Fields<'_>, i: usize) -> Result<()> {
        let p = format!("layers[{i}]");
        let version = self.version;
        self.locks.push(load_layer(&mut self.doc, f, &p, version).map_err(|e| {
            let name = f.text(&format!("{p}.name")).unwrap_or_default();
            e.in_context(format!("{p}「{name}」をcoreにできません"))
        })?);
        self.ids.push(LayerId(core_id(f.guid(&format!("{p}.id"))?)));
        self.parents.push(if version >= 6 {
            f.guid(&format!("{p}.parent"))?
        } else {
            [0; 16]
        });
        Ok(())
    }
    /// 親・保存した ID・ロックを付けて終える（`head` は文書の ID を含む頭の項目）。
    pub(crate) fn finish(self, head: &Fields<'_>) -> Result<Document> {
        let Self {
            doc,
            ids,
            parents,
            locks,
            ..
        } = self;
        let mut doc = doc;
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
                            .ok_or_else(|| Error::InvalidData("親グループがありません".into()))
                    })
                    .transpose()
            })
            .collect::<Result<_>>()?;
        if parents.iter().any(Option::is_some) {
            doc.set_structure_for_load(&parents)?;
        }
        let mut doc = doc.with_persistent_ids(core_id(head.guid("id")?), &ids)?;
        // ロックは読み終えてから付ける（読み手自身の画素・属性の設定をロックが断らないように。C# の `SetLocksForLoad` と同じ）。
        // 付けるのは履歴を持たない読み込みの経路で、ロックは合成を変えない
        for (id, locks) in ids.iter().zip(locks) {
            if locks != LayerLocks::NONE {
                doc.set_locks_for_load(*id, locks)?;
            }
        }
        Ok(doc)
    }
}

/// 文書の正本の版（使う機能で決まる）: グラデーションマップの混色（混色モード・混合率曲線）があれば 25、Rust 版だけの色調補正（種類 64〜69）が
/// あれば 24、Rust 版だけの Generator の種類があれば 23、ユーザーチャンネルだけなら 22、どれも無ければ Unity 版と同じ 21。
pub(crate) fn version_of(doc: &Document) -> i32 {
    let user = doc.channels().into_iter().any(|c| !c.is_standard());
    if uses_gradient_mixing(doc) {
        MIXING_VERSION
    } else if uses_rust_only_adjustments(doc) {
        ADJUST_VERSION
    } else if uses_rust_only_generators(doc) {
        PROCEDURAL_VERSION
    } else if !user {
        UNITY_NATIVE_VERSION
    } else {
        USER_CHANNELS_VERSION
    }
}
/// .ylp の正本（テクスチャセット 1 枚の文書）の辺の上限（画素）。読み手も書き手も同じ値。PSD などの取り込みは、保存できない大きさの
/// 文書を作らないよう、この値で断る。
pub const MAX_DOCUMENT_EDGE: u32 = 8192;
/// .ylp の正本の層の数の上限（グループも数える）。
pub const MAX_DOCUMENT_LAYERS: usize = 2048;

/// 層の入れ子が `yolu_core::MAX_GROUP_DEPTH` 以内か（1 回なめる）。
fn nesting_within_limit(doc: &Document) -> bool {
    let mut depth: HashMap<LayerId, usize> = HashMap::with_capacity(doc.layers().len());
    for l in doc.layers().iter().rev() {
        let d = l.parent().map_or(0, |p| depth.get(&p).map_or(0, |d| d + 1));
        if l.is_group() && d >= yolu_core::MAX_GROUP_DEPTH {
            return false;
        }
        depth.insert(l.id(), d);
    }
    true
}

/// 書く前の確かめ（`from_core` と、流して書く正本の両方）。
pub(crate) fn check_writable(doc: &Document) -> Result<()> {
    check(!doc.has_active_stroke(), "描画中のストロークがあります")?;
    // 手動の ID の色（正本の版 19）はまだ書けない。黙って落とさず、空でなければ断る
    if !doc.id_colors().colors().is_empty() {
        return Err(Error::Unwritable(Unwritable::ManualIdColors));
    }
    check_budget(
        doc.width() <= MAX_DOCUMENT_EDGE && doc.height() <= MAX_DOCUMENT_EDGE,
        "キャンバスの辺の上限は8192です",
    )?;
    check(
        (8..=512).contains(&doc.tile_size()) && doc.tile_size().is_power_of_two(),
        "正本のタイル寸法は8〜512の2の累乗です",
    )?;
    check_budget(
        doc.layers().len() <= MAX_DOCUMENT_LAYERS,
        "レイヤーの数の上限は2048です",
    )?;
    check_budget(
        nesting_within_limit(doc),
        format!("グループの入れ子の上限は{}段です", yolu_core::MAX_GROUP_DEPTH),
    )
}
/// 文書を正本の並び（中の版 `version`。C# の `DocumentBinary.Write` と同じ並び）で `sink` へ書く。層の始まりごとに `Sink::layer` を呼ぶ。
pub(crate) fn write_document(sink: &mut dyn Sink, doc: &Document, version: i32) -> Result<()> {
    write_head(sink, doc, version)?;
    for (i, layer) in doc.layers().iter().enumerate() {
        sink.layer(i)?;
        write_layer_to(sink, layer, version)?;
    }
    Ok(())
}
/// 識別子から層の数まで（層より前）。
pub(crate) fn write_head(sink: &mut dyn Sink, doc: &Document, version: i32) -> Result<()> {
    let mut w = Out {
        sink,
        mixing: version >= MIXING_VERSION,
    };
    w.raw(b"DOTPAINT")?;
    w.int(version)?;
    write_head_after_version(&mut w, doc, version)
}
/// 1 つの層。
pub(crate) fn write_layer_to(sink: &mut dyn Sink, layer: &yolu_core::Layer, version: i32) -> Result<()> {
    let mut w = Out {
        sink,
        mixing: version >= MIXING_VERSION,
    };
    write_layer(&mut w, layer)
}
/// 版の後ろから層の数まで（ID・寸法・Normal の設定・ユーザーチャンネル・層の数）。
fn write_head_after_version(w: &mut Out<'_>, doc: &Document, version: i32) -> Result<()> {
    let user: Vec<Channel> = doc
        .channels()
        .into_iter()
        .filter(|c| !c.is_standard())
        .collect();
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
    if version >= USER_CHANNELS_VERSION {
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
            w.value(&info.default.to_array())?;
        }
    }
    w.int(doc.layers().len() as i32)
}

/// 文書が Rust 版だけの Generator の種類（ノイズ・グランジ）の段を持つか（層の内容とマスクのスタック。無効な段も数える。
/// 持っていれば正本の版は 23 になり、Unity 版は開けない）。
pub(crate) fn uses_rust_only_generators(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        l.filters()
            .iter()
            .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
            .any(|e| {
                e.settings()
                    .generator_settings()
                    .is_some_and(|g| g.kind.is_procedural())
            })
    })
}
/// 文書が、混色（Standard 以外のモード）か混合率曲線を使うグラデーションマップ（調整の層か、層の内容・マスクのフィルターの段。無効な段も数える）を
/// 持つか。持っていれば正本の版は 25 になり、Unity 版は開けない。
pub(crate) fn uses_gradient_mixing(doc: &Document) -> bool {
    let mixes = |c: Option<ColorAdjust>| {
        matches!(c, Some(ColorAdjust::GradientMap(g)) if g.ramp().uses_mixing())
    };
    doc.layers().iter().any(|l| {
        l.adjustment().is_some_and(|a| mixes(a.color_adjust()))
            || l.filters()
                .iter()
                .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
                .any(|e| mixes(e.settings().color_adjust()))
    })
}
/// 文書が Rust 版だけの色調補正（調整の層の種類 64〜69、層の内容とマスクのフィルターの段の種類 64〜69。無効な段も数える）を持つか。
/// 持っていれば正本の版は 24 になり、Unity 版は開けない。
pub(crate) fn uses_rust_only_adjustments(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        l.adjustment().is_some_and(|a| a.kind().is_rust_only())
            || l.filters()
                .iter()
                .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
                .any(|e| e.settings().color_adjust().is_some())
    })
}
/// 1 つの層を core に足す（C# の読み手と同じ順: 種類で作り、属性、チャンネルごとの合成、画素、マスク）。返すのは、読み終えてから
/// 付けるロック（属性の印のビット 1 が立っていれば、直後の int）。
fn load_layer(doc: &mut Document, f: &Fields<'_>, p: &str, version: i32) -> Result<LayerLocks> {
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
                2 => AdjustmentSettings::hue_saturation(v(5)?, v(6)?, v(7)?)?,
                t => read_color_adjust(f, &format!("{a}.detail"), t)?.into_settings(),
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
            .ok_or_else(|| Error::InvalidData(format!("合成モード {blend} は範囲外です")))?,
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
    let locks = if attributes & 2 != 0 {
        let bits = f.int(&format!("{p}.locks"))?;
        u8::try_from(bits)
            .ok()
            .and_then(|b| LayerLocks::from_bits(b).ok())
            .filter(|l| *l != LayerLocks::NONE)
            .ok_or_else(|| Error::InvalidData(format!("{p}.locks {bits} は範囲外です")))?
    } else {
        LayerLocks::NONE
    };
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
                        .ok_or_else(|| Error::InvalidData(format!("{b}.mode {m} は範囲外です")))?,
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
                .map_err(|e| Error::from(e).in_context(format!("{tile}を変換できません")))?;
        }
        // 画素の無いチャンネルも面を持つ（C# の GetChannel）。有効の印は保存した値に
        let has_surface = doc.layer(id).is_some_and(|l| l.surface(c).is_some());
        if !has_surface {
            doc.set_channel_enabled(id, c, true)?;
        }
        doc.set_channel_enabled(id, c, f.boolean(&format!("{ch}.enabled"))?)?;
    }
    let has_mask = version >= 2 && f.boolean(&format!("{p}.has_mask"))?;
    if has_mask {
        doc.add_layer_mask(id)?;
        doc.set_layer_mask_enabled(id, f.boolean(&format!("{p}.mask.enabled"))?)?;
        doc.set_layer_mask_inverted(id, f.boolean(&format!("{p}.mask.inverted"))?)?;
        doc.set_layer_mask_density(id, f.float(&format!("{p}.mask.density"))?, false)?;
        for t in 0..f.int(&format!("{p}.mask.tile_count"))? {
            let tile = format!("{p}.mask.tiles[{t}]");
            let coord = tile_coord(f, &tile)?;
            doc.import_mask_tile(id, coord, f.bytes(&format!("{tile}.rgba"))?)
                .map_err(|e| Error::from(e).in_context(format!("{tile}を変換できません")))?;
        }
    }
    // 効果（C# の読み手と同じ順: 塗りつぶしの画像と投影、グラデーション、フィルター（内容、次にマスク）、Anchor）
    if attributes & 8 != 0 {
        let n = f.int(&format!("{p}.image_count"))?;
        let mut images = Vec::new();
        for k in 0..n {
            let image = format!("{p}.images[{k}]");
            images.push((
                f.channel(&format!("{image}.channel"))?,
                ImageId(core_id(f.guid(&format!("{image}.resource_id"))?)),
            ));
        }
        let projection = read_projection(f, &format!("{p}.projection"))?;
        doc.set_fill_images_for_load(id, &images, projection)
            .map_err(|e| Error::from(e).in_context("塗りつぶしの画像・投影をcoreにできません"))?;
    }
    if attributes & 32 != 0 {
        let mut gradients = Vec::new();
        for k in 0..f.int(&format!("{p}.gradient_count"))? {
            let g = format!("{p}.gradients[{k}]");
            gradients.push((f.channel(&format!("{g}.channel"))?, read_generator(f, &g)?));
        }
        doc.set_fill_gradients_for_load(id, gradients)
            .map_err(|e| Error::from(e).in_context("塗りつぶしのグラデーションをcoreにできません"))?;
    }
    if version >= 8 && f.boolean(&format!("{p}.has_surface_path"))? {
        let path = read_path(f, &format!("{p}.surface_path"), true, version)?;
        doc.set_path_for_load(id, path)
            .map_err(|e| Error::from(e).in_context("パスをcoreにできません"))?;
    }
    if version >= 9 && f.boolean(&format!("{p}.has_filters"))? {
        let specs = read_filters(f, &format!("{p}.filters"), true)?;
        doc.set_filters_for_load(id, FilterTarget::Content, specs)
            .map_err(|e| Error::from(e).in_context("フィルターをcoreにできません"))?;
        if has_mask {
            let specs = read_filters(f, &format!("{p}.mask.filters"), false)?;
            doc.set_filters_for_load(id, FilterTarget::Mask, specs)
                .map_err(|e| Error::from(e).in_context("マスクのフィルターをcoreにできません"))?;
        }
    }
    if version >= 10 && f.boolean(&format!("{p}.has_canvas_path"))? {
        let path = read_path(f, &format!("{p}.canvas_path"), false, version)?;
        doc.set_path_for_load(id, path)
            .map_err(|e| Error::from(e).in_context("パスをcoreにできません"))?;
    }
    if attributes & 16 != 0 {
        let flags = f.byte(&format!("{p}.anchor_flags"))?;
        for (bit, path, placement) in [
            (1, "anchor", AnchorPlacement::Layer),
            (2, "mask.anchor", AnchorPlacement::Mask),
        ] {
            if flags & bit != 0 {
                let a = format!("{p}.{path}");
                doc.set_anchor_for_load(
                    id,
                    placement,
                    AnchorId(core_id(f.guid(&format!("{a}.id"))?)),
                    f.text(&format!("{a}.name"))?,
                )
                .map_err(|e| Error::from(e).in_context("Anchorをcoreにできません"))?;
            }
        }
    }
    Ok(locks)
}

/// 2D・3D のパス（`{p}` の下の項目）。ブラシは丸いブラシの設定（半径は 2D では画素、3D ではモデルの空間）。
fn read_path(f: &Fields<'_>, p: &str, surface: bool, version: i32) -> Result<LayerPath> {
    let id = core_id(f.guid(&format!("{p}.id"))?);
    let channel = f.channel(&format!("{p}.channel"))?;
    let b = |name: &str| f.float(&format!("{p}.brush.{name}"));
    let flag = |name: &str| f.boolean(&format!("{p}.brush.{name}"));
    let color = f.rgba(&format!("{p}.brush.rgba"))?;
    let brush = paths::PathBrush(BrushSettings {
        radius: b("radius")?,
        hardness: b("hardness")?,
        spacing: b("spacing")?,
        opacity: b("opacity")?,
        flow: b("flow")?,
        color,
        erase: flag("erase")?,
        pressure_size: flag("pressure_size")?,
        pressure_opacity: flag("pressure_opacity")?,
        pressure_flow: flag("pressure_flow")?,
    });
    let count = f.int(&format!("{p}.point_count"))?;
    let material = if version >= 18 {
        let n = f.byte(&format!("{p}.material_count"))?;
        (n > 0)
            .then(|| {
                (0..n)
                    .map(|k| {
                        Ok(paths::ChannelPaint {
                            channel: f.channel(&format!("{p}.material[{k}].channel"))?,
                            color: f.rgba(&format!("{p}.material[{k}].rgba"))?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?
    } else {
        None
    };
    let pressure = |k: i32| f.float(&format!("{p}.points[{k}].pressure"));
    let bad = |e: paths::Error| Error::InvalidData(e.to_string());
    Ok(if surface {
        let mut points = Vec::new();
        for k in 0..count {
            points.push(paths::PathPoint {
                triangle: u32::try_from(f.int(&format!("{p}.points[{k}].triangle"))?)
                    .map_err(|_| Error::InvalidData("三角形の番号が負です".into()))?,
                u: f.float(&format!("{p}.points[{k}].u"))?,
                v: f.float(&format!("{p}.points[{k}].v"))?,
                pressure: pressure(k)?,
            });
        }
        let path = paths::SurfacePath {
            id,
            channel,
            brush,
            points,
            model_fingerprint: f.text(&format!("{p}.model_fingerprint"))?.to_owned(),
            material,
        };
        // 重心座標の -1e-9 以上 0 未満は 0 に丸める（C# の読み手は点を作るとき丸める）
        let mut path = path;
        for q in &mut path.points {
            *q = paths::PathPoint::new(q.triangle, q.u, q.v, q.pressure).map_err(bad)?;
        }
        LayerPath::Surface(path)
    } else {
        let mut points = Vec::new();
        for k in 0..count {
            points.push(
                paths::CanvasPoint::new(
                    f.float(&format!("{p}.points[{k}].x"))?,
                    f.float(&format!("{p}.points[{k}].y"))?,
                    pressure(k)?,
                )
                .map_err(bad)?,
            );
        }
        LayerPath::Canvas(paths::CanvasPath {
            id,
            channel,
            brush,
            points,
            material,
        })
    })
}

/// 2D・3D のパス（C# の `WritePath`・`WriteCanvasPath`）。
fn write_path(w: &mut Out<'_>, path: &LayerPath) -> Result<()> {
    w.int(paths::ALGORITHM_VERSION as i32)?;
    w.raw(&native_id(path.id()))?;
    w.int(path.channel().index() as i32)?;
    let (brush, material) = match path {
        LayerPath::Surface(p) => {
            w.text(&p.model_fingerprint)?;
            (p.brush.0, p.material.as_deref())
        }
        LayerPath::Canvas(p) => (p.brush.0, p.material.as_deref()),
    };
    for v in [
        brush.radius,
        brush.hardness,
        brush.spacing,
        brush.opacity,
        brush.flow,
    ] {
        w.float(v)?;
    }
    w.value(&brush.color.to_array())?;
    for v in [
        brush.erase,
        brush.pressure_size,
        brush.pressure_opacity,
        brush.pressure_flow,
    ] {
        w.boolean(v)?;
    }
    match path {
        LayerPath::Surface(p) => {
            w.int(p.points.len() as i32)?;
            for q in &p.points {
                w.int(q.triangle as i32)?;
                w.float(q.u)?;
                w.float(q.v)?;
                w.float(q.pressure)?;
            }
        }
        LayerPath::Canvas(p) => {
            w.int(p.points.len() as i32)?;
            for q in &p.points {
                w.float(q.x)?;
                w.float(q.y)?;
                w.float(q.pressure)?;
            }
        }
    }
    let material = material.unwrap_or_default();
    w.byte(material.len() as u8)?;
    for m in material {
        w.int(m.channel.index() as i32)?;
        w.value(&m.color.to_array())?;
    }
    Ok(())
}

/// 塗りつぶしの投影（`{p}` の下の項目。デカールだけが末尾に面の向きの項目を持つ）。
fn read_projection(f: &Fields<'_>, p: &str) -> Result<Projection> {
    let mode = f.int(&format!("{p}.mode"))?;
    let wrap = f.int(&format!("{p}.wrap"))?;
    let v = |name: &str| f.float(&format!("{p}.{name}"));
    let q = |name: &str| f.float(&format!("{p}.placement.{name}"));
    let mut projection = Projection {
        mode: ProjectionMode::try_from(u8::try_from(mode).map_err(|_| Error::InvalidData("投影の種類".into()))?)
            .map_err(|e| Error::InvalidData(e.to_string()))?,
        wrap: Wrap::try_from(u8::try_from(wrap).map_err(|_| Error::InvalidData("投影の外側".into()))?)
            .map_err(|e| Error::InvalidData(e.to_string()))?,
        tiles: [v("tile_u")?, v("tile_v")?],
        offset: [v("offset_u")?, v("offset_v")?],
        rotation: v("rotation")?,
        blend_width: v("blend_width")?,
        placement: Placement {
            center: [q("center_x")?, q("center_y")?, q("center_z")?],
            rotation: [q("rotation_x")?, q("rotation_y")?, q("rotation_z")?],
            size: [q("size_x")?, q("size_y")?, q("size_z")?],
        },
        ..Projection::default()
    };
    if projection.mode == ProjectionMode::Decal {
        projection.depth_hardness = v("depth_hardness")?;
        projection.backface_angle = v("backface_angle")?;
        projection.backface_hardness = v("backface_hardness")?;
    }
    Ok(projection)
}

/// Generator の設定（`{p}` の下の項目。形のグラデーションは形とランプ、ID の色は許容と色、Anchor は参照が続く）。
fn read_generator(f: &Fields<'_>, p: &str) -> Result<generator::Settings> {
    let kind = match f.int(&format!("{p}.type"))? {
        0 => generator::Kind::EdgeWear,
        1 => generator::Kind::Dirt,
        2 => generator::Kind::PositionGradient,
        3 => generator::Kind::Thickness,
        4 => generator::Kind::Direction,
        5 => generator::Kind::ShapeGradient,
        6 => generator::Kind::IdColor,
        64 => generator::Kind::Noise,
        65 => generator::Kind::Grunge,
        _ => generator::Kind::Anchor,
    };
    let mut g = generator::Settings::new(kind);
    let v = |name: &str| f.float(&format!("{p}.{name}"));
    g.low = v("low")?;
    g.high = v("high")?;
    g.softness = v("softness")?;
    g.invert = f.boolean(&format!("{p}.invert"))?;
    g.noise_amount = v("noise_amount")?;
    g.noise_scale = v("noise_scale")?;
    g.noise_seed = f.int(&format!("{p}.noise_seed"))?;
    g.noise_space = if f.int(&format!("{p}.noise_space"))? == 0 {
        generator::NoiseSpace::Model
    } else {
        generator::NoiseSpace::Uv
    };
    g.blend = match f.int(&format!("{p}.blend"))? {
        0 => generator::Blend::Multiply,
        1 => generator::Blend::Replace,
        2 => generator::Blend::Screen,
        3 => generator::Blend::Max,
        4 => generator::Blend::Min,
        5 => generator::Blend::Add,
        _ => generator::Blend::Subtract,
    };
    g.balance = v("balance")?;
    g.axis = usize::try_from(f.int(&format!("{p}.axis"))?).map_err(|_| Error::InvalidData("軸".into()))?;
    g.direction = [v("direction_x")?, v("direction_y")?, v("direction_z")?];
    g.use_bent_normal = f.boolean(&format!("{p}.bent_normal"))?;
    for k in 0..f.int(&format!("{p}.pin_count"))? {
        let pin = format!("{p}.pins[{k}]");
        let map = f.int(&format!("{pin}.kind"))?;
        let map = map_kind(map).ok_or_else(|| Error::InvalidData(format!("{pin}.kind {map} は範囲外です")))?;
        g.pins
            .insert(map, f.text(&format!("{pin}.key"))?.to_owned());
    }
    if kind == generator::Kind::ShapeGradient {
        let q = |name: &str| f.float(&format!("{p}.volume.{name}"));
        g.volume = generator::Volume {
            shape: match f.int(&format!("{p}.volume.shape"))? {
                0 => generator::Shape::Box,
                1 => generator::Shape::Sphere,
                _ => generator::Shape::Plane,
            },
            center: [q("center_x")?, q("center_y")?, q("center_z")?],
            rotation: [q("rotation_x")?, q("rotation_y")?, q("rotation_z")?],
            size: [q("size_x")?, q("size_y")?, q("size_z")?],
            falloff: q("falloff")?,
        };
        if f.int(&format!("{p}.algorithm"))? == 2 {
            g.ramp = Some(read_ramp(f, &format!("{p}.ramp"))?);
        }
    }
    if kind == generator::Kind::IdColor {
        g.id_tolerance = u8::try_from(f.int(&format!("{p}.tolerance"))?)
            .map_err(|_| Error::InvalidData("ID の色の許容".into()))?;
        for k in 0..f.int(&format!("{p}.color_count"))? {
            g.id_colors.push(
                u32::try_from(f.int(&format!("{p}.colors[{k}]"))?)
                    .map_err(|_| Error::InvalidData("ID の色".into()))?,
            );
        }
    }
    if kind == generator::Kind::Anchor {
        g.anchor = anchor::Reference {
            id: core_id(f.guid(&format!("{p}.anchor_id"))?),
            channel: f.channel(&format!("{p}.anchor_channel"))?,
            read: if f.int(&format!("{p}.anchor_read"))? == 0 {
                anchor::ReadMode::Value
            } else {
                anchor::ReadMode::Coverage
            },
        };
        // 参照が空（まだ選んでいない）は ID 0
        if f.guid(&format!("{p}.anchor_id"))? == [0; 16] {
            g.anchor.id = 0;
        }
    }
    if kind.is_procedural() {
        g.procedural = read_procedural(f, &format!("{p}.procedural"), kind)?;
    }
    Ok(g)
}

/// ノイズ・グランジの欄（正本の版 23。`write_generator` の末尾と対）。
fn read_procedural(
    f: &Fields<'_>,
    p: &str,
    kind: generator::Kind,
) -> Result<generator::Procedural> {
    use generator::{CellOutput, FractalMode, GrungePreset, NoiseBasis, ProceduralSpace};
    let int = |name: &str| f.int(&format!("{p}.{name}"));
    let float = |name: &str| f.float(&format!("{p}.{name}"));
    let bad = |name: &str| Error::InvalidData(format!("{p}.{name} は範囲外です"));
    let mut out = generator::Procedural {
        space: ProceduralSpace::from_index(i64::from(int("space")?)).ok_or_else(|| bad("space"))?,
        scale: float("scale")?,
        seed: int("seed")?,
        rotation: [
            float("rotation_x")?,
            float("rotation_y")?,
            float("rotation_z")?,
        ],
        bleed: float("bleed")?,
        blend_width: float("blend_width")?,
        ..generator::Procedural::default()
    };
    if kind == generator::Kind::Noise {
        out.basis = NoiseBasis::from_index(i64::from(int("basis")?)).ok_or_else(|| bad("basis"))?;
        out.cell_output = CellOutput::from_index(i64::from(int("cell_output")?))
            .ok_or_else(|| bad("cell_output"))?;
        out.fractal =
            FractalMode::from_index(i64::from(int("fractal")?)).ok_or_else(|| bad("fractal"))?;
        out.octaves = u32::try_from(int("octaves")?).map_err(|_| bad("octaves"))?;
        out.lacunarity = float("lacunarity")?;
        out.gain = float("gain")?;
    } else {
        out.preset =
            GrungePreset::from_index(i64::from(int("preset")?)).ok_or_else(|| bad("preset"))?;
    }
    Ok(out)
}
fn map_kind(index: i32) -> Option<MapKind> {
    use MapKind::*;
    [
        WorldNormal,
        Position,
        AmbientOcclusion,
        Curvature,
        Thickness,
        TangentNormal,
        Height,
        Id,
        BentNormal,
        Opacity,
    ]
    .get(usize::try_from(index).ok()?)
    .copied()
}

/// 値のカーブ（`{p}_count` と `{p}[i].x・y`。`write_curve` と対）。
fn read_curve(f: &Fields<'_>, p: &str) -> Result<Curve> {
    let mut points = Vec::new();
    for k in 0..f.int(&format!("{p}_count"))? {
        let c = format!("{p}[{k}]");
        points.push(CurvePoint {
            x: f.float(&format!("{c}.x"))?,
            y: f.float(&format!("{c}.y"))?,
        });
    }
    Curve::new(points).map_err(|e| Error::InvalidData(e.to_string()))
}

/// 64 からの調整・フィルターの種類の欄（正本の版 24。`write_color_adjust` と対）。知らない種類は断る。
fn read_color_adjust(f: &Fields<'_>, p: &str, kind: i32) -> Result<ColorAdjust> {
    let invalid = |e: yolu_core::CoreError| Error::InvalidData(e.to_string());
    let float = |name: &str| f.float(&format!("{p}.{name}"));
    let int = |name: &str| f.int(&format!("{p}.{name}"));
    Ok(match AdjustmentType::from_index(i64::from(kind)) {
        Some(AdjustmentType::GradientMap) => {
            let mut ramp = read_ramp(f, &format!("{p}.ramp"))?;
            // 混色の欄（正本の版 25）は、あるときだけ読む（版 24 までの文書には無い）
            if f.has(&format!("{p}.mix")) {
                ramp = read_gradient_mixing(f, p, &ramp)?;
            }
            ColorAdjust::GradientMap(GradientMap::new(ramp, f.boolean(&format!("{p}.reverse"))?))
        }
        Some(AdjustmentType::ToneCurve) => ColorAdjust::ToneCurve(ToneCurves::new(
            read_curve(f, &format!("{p}.composite"))?,
            read_curve(f, &format!("{p}.red"))?,
            read_curve(f, &format!("{p}.green"))?,
            read_curve(f, &format!("{p}.blue"))?,
        )),
        Some(AdjustmentType::ColorBalance) => {
            let mut values = [[0.0; 3]; 3];
            for (range, name) in ["shadows", "midtones", "highlights"].iter().enumerate() {
                for (axis, label) in ["cyan_red", "magenta_green", "yellow_blue"]
                    .iter()
                    .enumerate()
                {
                    values[range][axis] = float(&format!("{name}_{label}"))?;
                }
            }
            ColorAdjust::ColorBalance(
                ColorBalance::new(
                    values[0],
                    values[1],
                    values[2],
                    f.boolean(&format!("{p}.preserve_luminosity"))?,
                )
                .map_err(invalid)?,
            )
        }
        Some(AdjustmentType::BrightnessContrast) => ColorAdjust::BrightnessContrast(
            BrightnessContrast::new(float("brightness")?, float("contrast")?).map_err(invalid)?,
        ),
        Some(AdjustmentType::Threshold) => ColorAdjust::Threshold(
            Threshold::new(u32::try_from(int("level")?).unwrap_or(0)).map_err(invalid)?,
        ),
        Some(AdjustmentType::Posterize) => ColorAdjust::Posterize(
            Posterize::new(u32::try_from(int("levels")?).unwrap_or(0)).map_err(invalid)?,
        ),
        _ => {
            return Err(Error::InvalidData(format!(
                "{p} の種類 {kind} は色調補正ではありません"
            )))
        }
    })
}

/// グラデーションマップの混色の欄（正本の版 25。`write_gradient_mixing` と対）を、ランプへ足す。
fn read_gradient_mixing(f: &Fields<'_>, p: &str, ramp: &Ramp) -> Result<Ramp> {
    let invalid = |what: &str| Error::InvalidData(format!("{p}.{what} が範囲外です"));
    let mode = MixMode::from_index(i64::from(f.int(&format!("{p}.mix"))?))
        .ok_or_else(|| invalid("mix"))?;
    let correction = LuminanceCorrection::from_index(i64::from(f.int(&format!("{p}.luminance"))?))
        .ok_or_else(|| invalid("luminance"))?;
    let mut segments = Vec::new();
    for k in 0..f.int(&format!("{p}.segment_count"))? {
        let s = format!("{p}.segments[{k}]");
        segments.push(if f.boolean(&format!("{s}.enabled"))? {
            Some(read_curve(f, &format!("{s}.curve"))?)
        } else {
            None
        });
    }
    ramp.with_mixing(mode, correction)
        .with_segment_curves(segments)
        .map_err(|e| Error::InvalidData(e.to_string()))
}

fn read_ramp(f: &Fields<'_>, p: &str) -> Result<Ramp> {
    let mut colors = Vec::new();
    for k in 0..f.int(&format!("{p}.colors_count"))? {
        let c = format!("{p}.colors[{k}]");
        let rgb = f.bytes(&format!("{c}.rgb"))?;
        check(rgb.len() == 3, format!("{c}.rgb は 3 バイトではありません"))?;
        colors.push(ColorStop {
            position: f.float(&format!("{c}.position"))?,
            color: Rgba8::new(rgb[0], rgb[1], rgb[2], 255),
            midpoint: f.float(&format!("{c}.midpoint"))?,
        });
    }
    let mut opacities = Vec::new();
    for k in 0..f.int(&format!("{p}.opacities_count"))? {
        let o = format!("{p}.opacities[{k}]");
        opacities.push(OpacityStop {
            position: f.float(&format!("{o}.position"))?,
            opacity: f.float(&format!("{o}.opacity"))?,
            midpoint: f.float(&format!("{o}.midpoint"))?,
        });
    }
    let mut curve = Vec::new();
    for k in 0..f.int(&format!("{p}.curve_count"))? {
        let c = format!("{p}.curve[{k}]");
        curve.push(CurvePoint {
            x: f.float(&format!("{c}.x"))?,
            y: f.float(&format!("{c}.y"))?,
        });
    }
    Ramp::new(colors, opacities, Some(curve)).map_err(|e| Error::InvalidData(e.to_string()))
}

/// 1 つのスタック（`{p}.count` と `{p}.items[i]`）。段の種類・設定・チャンネルを読む。
fn read_filters(f: &Fields<'_>, p: &str, content: bool) -> Result<Vec<FilterSpec>> {
    let mut specs = Vec::new();
    for i in 0..f.int(&format!("{p}.count"))? {
        let item = format!("{p}.items[{i}]");
        let kind = f.int(&format!("{item}.type"))?;
        let float = |name: &str| f.float(&format!("{item}.{name}"));
        let uint = |name: &str| -> Result<u32> {
            u32::try_from(f.int(&format!("{item}.{name}"))?)
                .map_err(|_| Error::InvalidData(format!("{item}.{name} が負です")))
        };
        let settings = match kind {
            0 => EffectSettings::blur(uint("radius")?),
            1 => EffectSettings::sharpen(uint("radius")?, float("amount")?, uint("threshold")?),
            2 => EffectSettings::noise(
                float("amount")?,
                f.int(&format!("{item}.seed"))?,
                f.boolean(&format!("{item}.monochrome"))?,
            ),
            3 => EffectSettings::levels(
                float("input_black")?,
                float("input_white")?,
                float("gamma")?,
                float("output_black")?,
                float("output_white")?,
            ),
            4 => EffectSettings::invert(),
            5 => EffectSettings::normalize(),
            6 => EffectSettings::generator(read_generator(f, &format!("{item}.generator"))?),
            t => EffectSettings::from_color_adjust(read_color_adjust(
                f,
                &format!("{item}.adjust"),
                t,
            )?),
        };
        let channels = if content {
            let mut list = Vec::new();
            for k in 0..f.int(&format!("{item}.channel_count"))? {
                list.push(f.channel(&format!("{item}.channels[{k}].channel"))?);
            }
            Some(list)
        } else {
            None
        };
        let mut spec = FilterSpec::new(settings)
            .with_id(FilterId(core_id(f.guid(&format!("{item}.id"))?)))
            .strength(f.float(&format!("{item}.strength"))?);
        spec.enabled = f.boolean(&format!("{item}.enabled"))?;
        spec.channels = channels;
        specs.push(spec);
    }
    Ok(specs)
}

fn tile_coord(f: &Fields<'_>, tile: &str) -> Result<TileCoord> {
    let x = f.int(&format!("{tile}.x"))?;
    let y = f.int(&format!("{tile}.y"))?;
    Ok(TileCoord::new(
        u32::try_from(x).map_err(|_| Error::InvalidData(format!("{tile}.x が負です")))?,
        u32::try_from(y).map_err(|_| Error::InvalidData(format!("{tile}.y が負です")))?,
    ))
}

/// 正本のバイト列（512 MiB の予算を書くたびに確かめる）。
/// 正本の書き込み先。2 つ目は、混色の欄（正本の版 25）を書くか。
/// 正本の並びの書き先。平の値（整数・文字列・ID など）と `Bytes` の値（色・画素）を分けて受ける（版 26 は `Bytes` の値を部分へ書く）。
pub(crate) trait Sink {
    /// 平の値のバイト列。
    fn plain(&mut self, b: &[u8]) -> Result<()>;
    /// 小さな `Bytes` の値（色）。
    fn value(&mut self, b: &[u8]) -> Result<()>;
    /// タイル 1 枚の画素（`Bytes` の値。要るときだけ写す）。
    fn tile(&mut self, surface: &yolu_core::Surface, coord: TileCoord) -> Result<()>;
    /// `layers[i]` の始まり。
    fn layer(&mut self, _i: usize) -> Result<()> {
        Ok(())
    }
}
/// メモリへ全部を並べる（`from_core`。512 MiB まで）。
pub(crate) struct VecSink(pub Vec<u8>);
impl VecSink {
    fn grew(&self) -> Result<()> {
        check_budget(
            self.0.len() <= MAX_ENTRY_BYTES,
            "正本の512 MiB予算を超えています",
        )
    }
}
impl Sink for VecSink {
    fn plain(&mut self, b: &[u8]) -> Result<()> {
        self.0.extend_from_slice(b);
        self.grew()
    }
    fn value(&mut self, b: &[u8]) -> Result<()> {
        self.plain(b)
    }
    fn tile(&mut self, surface: &yolu_core::Surface, coord: TileCoord) -> Result<()> {
        let at = self.0.len();
        self.0.resize(at + surface.tile_bytes(), 0);
        surface.copy_tile(coord, &mut self.0[at..])?;
        self.grew()
    }
}
struct Out<'s> {
    sink: &'s mut dyn Sink,
    mixing: bool,
}
impl Out<'_> {
    fn raw(&mut self, b: &[u8]) -> Result<()> {
        self.sink.plain(b)
    }
    fn value(&mut self, b: &[u8]) -> Result<()> {
        self.sink.value(b)
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
        check_budget(v.len() <= 4096, "正本の文字列はUTF-8で4096バイトまでです")?;
        self.int(v.len() as i32)?;
        self.raw(v.as_bytes())
    }
    fn tiles(&mut self, surface: &yolu_core::Surface) -> Result<()> {
        self.int(surface.tile_count() as i32)?;
        let bytes = surface.tile_bytes() as i32;
        for coord in surface.tile_coords() {
            self.int(coord.x as i32)?;
            self.int(coord.y as i32)?;
            self.int(bytes)?;
            self.sink.tile(surface, coord)?;
        }
        Ok(())
    }
}

/// 1 つの層（C# の `DocumentBinary.Write` の層の並び）。
fn write_layer(w: &mut Out<'_>, layer: &yolu_core::Layer) -> Result<()> {
    let named = |why: &str| format!("層「{}」の{why}", layer.name());
    w.raw(&native_id(layer.id().0))?;
    w.text(layer.name())
        .map_err(|e| e.in_context(named("名前")))?;
    w.boolean(layer.visible())?;
    w.float(layer.opacity())?;
    w.int(layer.blend_mode() as i32)?;
    let blends: Vec<(Channel, ChannelBlend)> = layer.channel_blends().collect();
    let locks = layer.locks();
    let images: Vec<(Channel, ImageId)> = layer.fill_images().collect();
    let fill_images = layer.kind() == LayerKind::Fill
        && (!images.is_empty() || *layer.projection() != Projection::default());
    let mask_anchor = layer.mask().and_then(|m| m.anchor());
    let anchors = layer.anchor().is_some() || mask_anchor.is_some();
    let gradients: Vec<(Channel, &generator::Settings)> = layer.fill_gradients().collect();
    // 属性の印: ビット 0 クリッピング、ビット 1 ロックが続く、ビット 2 チャンネルごとの設定が続く、ビット 3 塗りつぶしの画像、ビット 4 Anchor、
    // ビット 5 塗りつぶしのグラデーション。ロックの印（int、0 は書かない）は属性の直後、チャンネルごとの設定より前
    w.byte(
        u8::from(layer.clipping())
            | if locks == LayerLocks::NONE { 0 } else { 2 }
            | if blends.is_empty() { 0 } else { 4 }
            | if fill_images { 8 } else { 0 }
            | if anchors { 16 } else { 0 }
            | if gradients.is_empty() { 0 } else { 32 },
    )?;
    if locks != LayerLocks::NONE {
        w.int(i32::from(locks.bits()))?;
    }
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
        w.value(&v.to_array())?;
    }
    if fill_images {
        w.int(images.len() as i32)?;
        for (c, id) in &images {
            w.int(c.index() as i32)?;
            w.raw(&native_id(id.0))?;
        }
        write_projection(w, layer.projection())?;
    }
    if !gradients.is_empty() {
        w.int(gradients.len() as i32)?;
        for (c, g) in &gradients {
            w.int(c.index() as i32)?;
            write_generator(w, g)?;
        }
    }
    if layer.kind() == LayerKind::Adjustment {
        let a = layer
            .adjustment()
            .ok_or_else(|| Error::InvalidData(named("調整の設定がありません")))?;
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
        // 64 からの種類（正本の版 24）は、8 つの値を既定のまま書いたあとに種類ごとの欄が続く
        if let Some(detail) = a.color_adjust() {
            write_color_adjust(w, &detail)?;
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
    let surface_path = layer.path().filter(|p| !p.is_canvas());
    w.boolean(surface_path.is_some())?;
    if let Some(path) = surface_path {
        write_path(w, path)?;
    }
    let mask_filters = layer.mask().map_or(&[][..], |m| m.filters());
    let filtered = !layer.filters().is_empty() || !mask_filters.is_empty();
    w.boolean(filtered)?;
    if filtered {
        write_filters(w, layer.filters(), true)?;
        if layer.mask().is_some() {
            write_filters(w, mask_filters, false)?;
        }
    }
    let canvas_path = layer.path().filter(|p| p.is_canvas());
    w.boolean(canvas_path.is_some())?;
    if let Some(path) = canvas_path {
        write_path(w, path)?;
    }
    if anchors {
        w.byte(u8::from(layer.anchor().is_some()) | if mask_anchor.is_some() { 2 } else { 0 })?;
        for a in [layer.anchor(), mask_anchor].into_iter().flatten() {
            w.raw(&native_id(a.id().0))?;
            w.text(a.name())?;
        }
    }
    Ok(())
}

fn write_projection(w: &mut Out<'_>, p: &Projection) -> Result<()> {
    w.int(Projection::ALGORITHM_VERSION as i32)?;
    w.int(p.mode as i32)?;
    w.int(p.wrap as i32)?;
    for v in [
        p.tiles[0],
        p.tiles[1],
        p.offset[0],
        p.offset[1],
        p.rotation,
        p.blend_width,
        p.placement.center[0],
        p.placement.center[1],
        p.placement.center[2],
        p.placement.rotation[0],
        p.placement.rotation[1],
        p.placement.rotation[2],
        p.placement.size[0],
        p.placement.size[1],
        p.placement.size[2],
    ] {
        w.float(v)?;
    }
    if p.mode == ProjectionMode::Decal {
        w.float(p.depth_hardness)?;
        w.float(p.backface_angle)?;
        w.float(p.backface_hardness)?;
    }
    Ok(())
}

fn write_generator(w: &mut Out<'_>, g: &generator::Settings) -> Result<()> {
    w.int(g.kind as i32)?;
    w.int(g.algorithm_version() as i32)?;
    for v in [g.low, g.high, g.softness] {
        w.float(v)?;
    }
    w.boolean(g.invert)?;
    w.float(g.noise_amount)?;
    w.float(g.noise_scale)?;
    w.int(g.noise_seed)?;
    w.int(g.noise_space as i32)?;
    w.int(g.blend as i32)?;
    w.float(g.balance)?;
    w.int(g.axis as i32)?;
    for v in g.direction {
        w.float(v)?;
    }
    w.boolean(g.use_bent_normal)?;
    w.int(g.pins.len() as i32)?;
    for (kind, key) in &g.pins {
        w.int(*kind as i32)?;
        w.text(key)?;
    }
    if g.kind == generator::Kind::ShapeGradient {
        w.int(g.volume.shape as i32)?;
        for v in g
            .volume
            .center
            .into_iter()
            .chain(g.volume.rotation)
            .chain(g.volume.size)
            .chain([g.volume.falloff])
        {
            w.float(v)?;
        }
        if let Some(r) = &g.ramp {
            // Unity 版と共有の並び。混色・混合率曲線は書けないので、黙って落とさず断る（画面は塗りつぶしのグラデーションには出さない）
            if r.uses_mixing() {
                return Err(Error::Unwritable(Unwritable::GeneratorRampMixing));
            }
            write_ramp(w, r)?;
        }
    }
    if g.kind == generator::Kind::IdColor {
        w.int(i32::from(g.id_tolerance))?;
        w.int(g.id_colors.len() as i32)?;
        for c in &g.id_colors {
            w.int(*c as i32)?;
        }
    }
    if g.kind == generator::Kind::Anchor {
        w.raw(&native_id(g.anchor.id))?;
        w.int(g.anchor.channel.index() as i32)?;
        w.int(g.anchor.read as i32)?;
    }
    if g.kind.is_procedural() {
        // ノイズ・グランジの欄（正本の版 23。`read_procedural` と対）
        let p = &g.procedural;
        w.int(p.space as i32)?;
        w.float(p.scale)?;
        w.int(p.seed)?;
        for r in p.rotation {
            w.float(r)?;
        }
        w.float(p.bleed)?;
        w.float(p.blend_width)?;
        if g.kind == generator::Kind::Noise {
            w.int(p.basis as i32)?;
            w.int(p.cell_output as i32)?;
            w.int(p.fractal as i32)?;
            w.int(p.octaves as i32)?;
            w.float(p.lacunarity)?;
            w.float(p.gain)?;
        } else {
            w.int(p.preset as i32)?;
        }
    }
    Ok(())
}

/// ランプ（色の分岐点・不透明度の分岐点・値のカーブ。`read_ramp` と対）。
fn write_ramp(w: &mut Out<'_>, r: &Ramp) -> Result<()> {
    w.int(r.colors().len() as i32)?;
    for c in r.colors() {
        w.float(c.position)?;
        w.value(&[c.color.r, c.color.g, c.color.b])?;
        w.float(c.midpoint)?;
    }
    w.int(r.opacities().len() as i32)?;
    for o in r.opacities() {
        w.float(o.position)?;
        w.float(o.opacity)?;
        w.float(o.midpoint)?;
    }
    write_curve(w, r.value_curve())
}
/// グラデーションマップの混色の欄（正本の版 25。`read_gradient_mixing` と対）: 混色モード・輝度の補正・区間の数と、区間ごとの印と混合率曲線。
fn write_gradient_mixing(w: &mut Out<'_>, r: &Ramp) -> Result<()> {
    w.int(i32::from(r.mix_mode().index()))?;
    w.int(i32::from(r.luminance_correction().index()))?;
    w.int(r.colors().len() as i32 - 1)?;
    for k in 0..r.colors().len() - 1 {
        match r.segment_curve(k) {
            Some(curve) => {
                w.boolean(true)?;
                write_curve(w, curve)?;
            }
            None => w.boolean(false)?,
        }
    }
    Ok(())
}
/// 値のカーブ（点の数と x・y。`read_curve` と対）。
fn write_curve(w: &mut Out<'_>, curve: &Curve) -> Result<()> {
    w.int(curve.points().len() as i32)?;
    for c in curve.points() {
        w.float(c.x)?;
        w.float(c.y)?;
    }
    Ok(())
}
/// 64 からの調整・フィルターの種類の欄（正本の版 24。`read_color_adjust` と対）。
fn write_color_adjust(w: &mut Out<'_>, value: &ColorAdjust) -> Result<()> {
    match value {
        ColorAdjust::GradientMap(v) => {
            w.boolean(v.reverse())?;
            write_ramp(w, v.ramp())?;
            if w.mixing {
                write_gradient_mixing(w, v.ramp())?;
            }
        }
        ColorAdjust::ToneCurve(v) => {
            for channel in [
                ToneChannel::Composite,
                ToneChannel::Red,
                ToneChannel::Green,
                ToneChannel::Blue,
            ] {
                write_curve(w, v.curve(channel))?;
            }
        }
        ColorAdjust::ColorBalance(v) => {
            for range in [
                BalanceRange::Shadows,
                BalanceRange::Midtones,
                BalanceRange::Highlights,
            ] {
                for value in v.values(range) {
                    w.float(value)?;
                }
            }
            w.boolean(v.preserve_luminosity())?;
        }
        ColorAdjust::BrightnessContrast(v) => {
            w.float(v.brightness())?;
            w.float(v.contrast())?;
        }
        ColorAdjust::Threshold(v) => w.int(v.level() as i32)?,
        ColorAdjust::Posterize(v) => w.int(v.levels() as i32)?,
    }
    Ok(())
}

/// 1 つのスタック（C# の `WriteFilters`）。Generator の段はフィルターの値を既定のまま書き、そのあとに Generator の欄が続く。
fn write_filters(w: &mut Out<'_>, stack: &[FilterEffect], content: bool) -> Result<()> {
    w.int(stack.len() as i32)?;
    for e in stack {
        w.raw(&native_id(e.id().0))?;
        w.int(e.settings().type_index())?;
        w.int(1)?; // アルゴリズムの版（段の種類ごと。どれも 1）
        w.boolean(e.enabled())?;
        w.float(e.strength())?;
        if content {
            w.int(e.channels().len() as i32)?;
            for c in e.channels() {
                w.int(c.index() as i32)?;
            }
        }
        // radius, amount, threshold, seed, monochrome, 入力・出力の範囲（使わない値は既定）
        let (mut radius, mut amount, mut threshold, mut seed, mut mono) =
            (0u32, 0.0, 0u32, 0, false);
        let mut levels = [0.0, 1.0, 1.0, 0.0, 1.0];
        match e.settings() {
            EffectSettings::Filter(f) => match *f {
                yolu_core::filter::Settings::GaussianBlur { radius: r } => radius = r,
                yolu_core::filter::Settings::Sharpen {
                    radius: r,
                    amount: a,
                    threshold: t,
                } => {
                    radius = r;
                    amount = a;
                    threshold = t;
                }
                yolu_core::filter::Settings::Noise {
                    amount: a,
                    seed: s,
                    monochrome: m,
                } => {
                    amount = a;
                    seed = s;
                    mono = m;
                }
                yolu_core::filter::Settings::Levels {
                    input_black,
                    input_white,
                    gamma,
                    output_black,
                    output_white,
                } => levels = [input_black, input_white, gamma, output_black, output_white],
                _ => {}
            },
            EffectSettings::Generator(_) => {}
        }
        w.int(radius as i32)?;
        w.float(amount)?;
        w.int(threshold as i32)?;
        w.int(seed)?;
        w.boolean(mono)?;
        for v in levels {
            w.float(v)?;
        }
        if let EffectSettings::Generator(g) = e.settings() {
            write_generator(w, g)?;
        }
        if let Some(detail) = e.settings().color_adjust() {
            write_color_adjust(w, &detail)?;
        }
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
            let error = native.to_core_within(Some(budget)).err().unwrap();
            // 壊れたファイルではなく予算超過として返す（画面が言い分ける）
            assert!(matches!(error, Error::Budget(_)), "{error:?}");
            let message = error.to_string();
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
