//! 正本（`NativeDocument`）と core の文書の行き来。意味は Unity 版の `DocumentBinary.Read` / `Write` と同じにする（C# が書いた正本は
//! core を通して書き戻すとバイト一致する）。範囲は M2 のレイヤー（レイヤーの種類・入れ子と通過・分離・ラスターマスク・クリッピング・レイヤーのロック（版 12）・
//! チャンネルごとの有効と合成（版 14）・Normal の出力の設定（版 7）・版 22 のユーザーチャンネル）と、効果（フィルターのスタックと Generator の段
//! （版 9・11・13・15）、Anchor（版 20）、塗りつぶしの画像と投影（版 16・17）、塗りつぶしのグラデーション（版 21））と、編集できる 2D・3D のパス
//! （版 8・10・18）、レイヤーの後の手動の ID の色（版 19）。core に無い項目は先に検査して断り、部分変換を返さない。
use crate::native::{
    ADJUST_VERSION, ANTI_ALIAS_VERSION, BAKE_PRIORITY_VERSION, EFFECTS_VERSION,
    MANUAL_ID_COLORS_VERSION, MIXING_VERSION, PATHS_VERSION, POINT_GRADIENT_VERSION,
    PROCEDURAL_VERSION, RULERS_VERSION, SEAMS_VERSION, TEXT_VERSION, UNITY_NATIVE_VERSION,
    USER_CHANNELS_VERSION,
};
use crate::{
    check, check_budget, Error, NativeDocument, NativeValue as V, Result, Unwritable,
    MAX_ENTRY_BYTES,
};
use std::collections::{BTreeMap, HashMap};
use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::fill_image::{Placement, Projection, ProjectionMode, Wrap};
use yolu_core::fill_points::{GradientPoint, PointGradient, PointSpace};
use yolu_core::generator::{
    self, anchor, ColorStop, LuminanceCorrection, MapKind, MixMode, OpacityStop, Ramp,
};
use yolu_core::glam::{DVec2, DVec3};
use yolu_core::mesh_maps::{IdColorAssignments, MeshOverlapPriority, MeshOverlapRule};
use yolu_core::paths;
use yolu_core::text::{TextAlign, TextFont, TextSettings};
use yolu_core::{
    AdjustmentSettings, AdjustmentType, AnchorId, AnchorPlacement, AntiAlias, BalanceRange,
    BlendMode, BrightnessContrast, BrushSettings, Channel, ChannelBlend, ChannelInfo, ChannelKind,
    ColorAdjust, ColorBalance, ColorSpace, Document, EffectSettings, FilterEffect, FilterId,
    FilterSpec, FilterTarget, GradientMap, HeightEdgeMode, ImageId, LayerId, LayerKind, LayerLocks,
    LayerPath, NormalSettings, NormalYDirection, Posterize, Rgba8, Ruler, RulerId, RulerKind,
    RulerPlace, RulerScope, Threshold, TileCoord, ToneChannel, ToneCurves,
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

/// core に無い項目なら、知らせる単位（レイヤーの機能ごとのパス）と理由。知らないパスも断る（読み手に足した項目を黙って通さない）。
fn unsupported(path: &str, fields: &HashMap<&str, &V>) -> Option<(String, &'static str)> {
    let Some(rest) = path.strip_prefix("layers[") else {
        let head = path.split(['.', '[']).next().unwrap_or(path);
        return match head {
            "magic" | "version" | "id" | "width" | "height" | "tile_size" | "normal"
            | "user_channel_count" | "user_channels" | "filter_seams" | "layer_count"
            | "manual_id_colors" => None,
            "bake_priority" => None,
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
        | "attributes_ext"
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
        | "gradients"
        | "paths"
        | "point_gradient_count"
        | "point_gradients"
        | "text"
        | "ruler_count"
        | "rulers" => None,
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
    /// core へ渡すと失われる項目（レイヤーの機能ごとに 1 つ、`layers[2].filters（フィルター・Generator）` の形）。空なら変換の対象
    /// （タイルの余白・画素の予算は変換時に検査する）。非表示・無効のレイヤーやマスクの中の項目も省略せずに断る。
    pub fn core_issues(&self) -> Vec<String> {
        core_issues_of(self.fields())
    }

    /// 編集用の core の文書にする（C# の `DocumentBinary.Read` と同じ意味）。文書とレイヤーの ID、透明の画素の RGB を保ち、読み込みを
    /// Undo の履歴に残さない。元の `NativeDocument` は変えない。
    pub fn to_core(&self) -> Result<Document> {
        self.to_core_within(None)
    }

    /// `to_core` の画素の予算を指定できる形（None は core の既定、`DEFAULT_SOURCE_BUDGET_BYTES`）。超えたら、どのレイヤーのどのタイルで
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
        load.finish(&f, self.fields())
    }

    /// core の文書から正本を作る（C# の `DocumentBinary.Write` と同じ並び）。ユーザーチャンネルが無ければ Unity 版と同じ版 21、あれば
    /// 版 22。履歴は保存しない。手動の ID の色はレイヤーの後の塊（版 19）に書く（空なら塊を書かず、版も変わらない）。正本の範囲外の寸法・タイル寸法・レイヤーの数・名前と、
    /// 進行中のストロークは断る。値の無い塗りつぶしのチャンネルとグループの有効の印は、合成に効かず C# の書き手も書かないので書かない。
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

/// 正本を core の文書へレイヤーごとに入れる（頭 → レイヤー → 終わり）。メモリの正本も、流して読む正本も同じ道を通る。
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
        if version >= BAKE_PRIORITY_VERSION {
            // 履歴の段を積む設定より先に戻す（戻すのは履歴が空のときだけ）
            doc.restore_bake_priority(read_bake_priority(f)?)?;
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
        if version >= SEAMS_VERSION {
            doc.set_filter_seams_for_load(f.boolean("filter_seams")?);
        }
        Ok(Self {
            doc,
            version,
            ids: Vec::new(),
            parents: Vec::new(),
            locks: Vec::new(),
        })
    }
    /// レイヤー `i` を足す（`f` はそのレイヤーの項目を含む）。
    pub(crate) fn layer(&mut self, f: &Fields<'_>, i: usize) -> Result<()> {
        let p = format!("layers[{i}]");
        let version = self.version;
        self.locks
            .push(load_layer(&mut self.doc, f, &p, version).map_err(|e| {
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
    /// 親・保存した ID・ロックと手動の ID の色を付けて終える（`head` は文書の ID を含む頭の項目、`tail` は読み終えた正本の項目で、
    /// レイヤーの後の手動の ID の色をここから読む。レイヤーごとに流して読む道は、読み終えた `NativeDocument` の項目を渡す）。
    pub(crate) fn finish(self, head: &Fields<'_>, tail: &[crate::NativeField]) -> Result<Document> {
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
        // 手動の ID の色は、結び付けたモデルの指紋が今のモデルと合うかに関わらず、書いてあったとおり文書へ戻す（合わなければ、使う側が
        // 「別のモデルのもの」として扱う。ここで捨てると、別のモデルを開いて保存したときに色が消える）。履歴は残さない
        let colors = id_colors_of(tail)?;
        if !colors.colors().is_empty() {
            doc.restore_id_colors(colors)?;
        }
        Ok(doc)
    }
}

/// 読み終えた正本の項目から、レイヤーの後の手動の ID の色（版 19）。塊が無ければ空。塊は項目の並びの最後にある。
pub(crate) fn id_colors_of(fields: &[crate::NativeField]) -> Result<IdColorAssignments> {
    const BLOCK: &str = "manual_id_colors.";
    let start = fields
        .iter()
        .rposition(|f| !f.path.starts_with(BLOCK))
        .map_or(0, |i| i + 1);
    let block = &fields[start..];
    if block.is_empty() {
        return Ok(IdColorAssignments::default());
    }
    let f = Fields::of(block);
    let count = f.int("manual_id_colors.count")?;
    let binding = f.text("manual_id_colors.binding")?.to_owned();
    let mut colors = BTreeMap::new();
    for i in 0..count {
        let p = format!("manual_id_colors.colors[{i}]");
        let number = |leaf: &str| {
            let v = f.int(&format!("{p}.{leaf}"))?;
            u32::try_from(v).map_err(|_| Error::InvalidData(format!("{p}.{leaf} が負です")))
        };
        colors.insert(number("part")? as usize, number("rgb")?);
    }
    IdColorAssignments::new(binding, colors)
        .map_err(|e| Error::InvalidData(format!("手動の ID の色を core の形にできません: {}", e.0)))
}

/// 版 33 の頭の `bake_priority` から、ベイクの優先（読み手が並び・範囲・指紋を確かめた後）。
fn read_bake_priority(f: &Fields<'_>) -> Result<MeshOverlapPriority> {
    let list = |name: &str| -> Result<std::collections::BTreeSet<usize>> {
        (0..f.int(&format!("bake_priority.{name}_count"))?)
            .map(|i| Ok(f.int(&format!("bake_priority.{name}[{i}]"))? as usize))
            .collect()
    };
    MeshOverlapPriority::new(
        MeshOverlapRule::from_index(f.int("bake_priority.rule")?)
            .ok_or_else(|| Error::InvalidData("ベイクの優先の決め方が不正です".into()))?,
        f.boolean("bake_priority.skip_outside")?,
        f.text("bake_priority.binding")?.to_owned(),
        list("skip")?,
        list("prefer")?,
    )
    .map_err(|e| Error::InvalidData(e.to_string()))
}

/// 版 35 の機能（定規を持つレイヤー、または線対称の 2D のパス）を使うか。
pub(crate) fn uses_rulers_version(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        !l.rulers().is_empty()
            || l.path()
                .into_iter()
                .chain(l.paths().iter().map(|e| &e.path))
                .any(|p| {
                    matches!(
                        p.style().symmetry,
                        paths::PathSymmetry::Canvas(c) if c.mode == yolu_core::SymmetryMode::Lines
                    )
                })
    })
}

/// パスのブラシにアンチエイリアスの段（なし でない）を持つパスがあるか（あれば版 34）。
fn uses_anti_aliased_paths(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        l.path()
            .into_iter()
            .chain(l.paths().iter().map(|e| &e.path))
            .any(|p| path_brush(p).anti_alias != AntiAlias::None)
    })
}

/// パスのブラシの設定。
fn path_brush(path: &LayerPath) -> BrushSettings {
    match path {
        LayerPath::Surface(p) => p.brush.0,
        LayerPath::Canvas(p) => p.brush.0,
    }
}

/// 文書の正本の版（使う機能で決まる）: 定規を持つレイヤー（グループ）か、線対称の 2D のパスがあれば 35（版 27〜34 の中身も読み書きできる版）、
/// パスのブラシのアンチエイリアス（なし 以外）があれば 34（版 27〜33 の中身も読み書きできる版）、
/// 重なった UV のベイクの優先を既定から変えていれば 33（版 27〜32 の中身も読み書きできる版）、
/// レイヤーのフィルターが UV の継ぎ目をまたぐ設定を切っていれば 32（版 27〜30 の中身も読み書きできる版）、
/// テキストレイヤーがあれば 30（版 27〜29 の中身も読み書きできる）、
/// 塗りつぶしの点のグラデーションか、異方性のフィルターを切った塗りつぶしの画像があれば 29（版 27・28 の中身も読み書きできる）、
/// 0.5.0 の効果（フィルターの段の種類 70〜79、Generator の種類 66・68・69・70）があれば 28（版 27 の中身も読み書きできる）、パスの一覧の形で書くパス
/// （塗りつぶしレイヤーのパス・2 本以上・名前・隠す・種類・筆先・深さ・対称・角・取っ手）があれば 27、グラデーションマップの混色（混色モード・混合率曲線）があれば 25、
/// Rust 版だけの色調補正（種類 64〜69）があれば 24、Rust 版だけの Generator の種類があれば 23、ユーザーチャンネルだけなら 22、どれも無ければ Unity 版と同じ 21。
/// 手動の ID の色（版 19 から）は 21 以上のどの版でも書けるので、版を決めない（色だけを持つ文書は Unity 版が読める 21 のまま）。
pub(crate) fn version_of(doc: &Document) -> i32 {
    let user = doc.channels().into_iter().any(|c| !c.is_standard());
    if uses_rulers_version(doc) {
        RULERS_VERSION
    } else if uses_anti_aliased_paths(doc) {
        ANTI_ALIAS_VERSION
    } else if !doc.bake_priority().is_default() {
        BAKE_PRIORITY_VERSION
    } else if !doc.filter_seams() {
        SEAMS_VERSION
    } else if doc.layers().iter().any(|l| l.text().is_some()) {
        TEXT_VERSION
    } else if uses_point_gradient_version(doc) {
        POINT_GRADIENT_VERSION
    } else if uses_image_generators(doc) || uses_new_filters(doc) {
        EFFECTS_VERSION
    } else if uses_path_lists(doc) {
        PATHS_VERSION
    } else if uses_gradient_mixing(doc) {
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
/// .ylp の正本のレイヤーの数の上限（グループも数える）。
pub const MAX_DOCUMENT_LAYERS: usize = 2048;

/// レイヤーの入れ子が `yolu_core::MAX_GROUP_DEPTH` 以内か（1 回なめる）。
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
        format!(
            "グループの入れ子の上限は{}段です",
            yolu_core::MAX_GROUP_DEPTH
        ),
    )
}
/// 文書を正本の並び（中の版 `version`。C# の `DocumentBinary.Write` と同じ並び）で `sink` へ書く。レイヤーの始まりごとに `Sink::layer` を呼ぶ。
pub(crate) fn write_document(sink: &mut dyn Sink, doc: &Document, version: i32) -> Result<()> {
    write_head(sink, doc, version)?;
    for (i, layer) in doc.layers().iter().enumerate() {
        sink.layer(i)?;
        write_layer_to(sink, layer, version)?;
    }
    write_tail(sink, doc, version)
}
/// レイヤーの後（手動の ID の色）。空なら何も書かず、塊の無い今の版のままにする。空でなければ版 19 の末尾の塊 `YLID`（`tag` は `Bytes` の値、
/// 残りは平の値。版 26 では `tag` が部分の側に入る）。版 21 以上の書き手の版はどれも 19 以上なので、色のために版を上げることは無い。
/// 色は `IdColorAssignments` が数・番号・色・指紋を検査済みで、番号は狭義の昇順で並ぶ。
pub(crate) fn write_tail(sink: &mut dyn Sink, doc: &Document, version: i32) -> Result<()> {
    let assigned = doc.id_colors();
    if assigned.colors().is_empty() {
        return Ok(());
    }
    check(
        version >= MANUAL_ID_COLORS_VERSION,
        "手動の ID の色は正本の版 19 から書けます",
    )?;
    let mut w = Out::for_version(sink, version);
    w.value(b"YLID")?;
    w.int(assigned.colors().len() as i32)?;
    w.text(assigned.binding())?;
    for (part, rgb) in assigned.colors() {
        w.int(*part as i32)?;
        w.int(*rgb as i32)?;
    }
    Ok(())
}
/// 識別子からレイヤーの数まで（レイヤーより前）。
pub(crate) fn write_head(sink: &mut dyn Sink, doc: &Document, version: i32) -> Result<()> {
    let mut w = Out::for_version(sink, version);
    w.raw(b"DOTPAINT")?;
    w.int(version)?;
    write_head_after_version(&mut w, doc, version)
}
/// 1 つのレイヤー。
pub(crate) fn write_layer_to(
    sink: &mut dyn Sink,
    layer: &yolu_core::Layer,
    version: i32,
) -> Result<()> {
    let mut w = Out::for_version(sink, version);
    write_layer(&mut w, layer)
}
/// 版の後ろからレイヤーの数まで（ID・寸法・Normal の設定・ユーザーチャンネル・レイヤーの数）。
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
    if version >= SEAMS_VERSION {
        w.boolean(doc.filter_seams())?;
    }
    if version >= BAKE_PRIORITY_VERSION {
        let p = doc.bake_priority();
        w.int(p.rule as i32)?;
        w.boolean(p.skip_outside)?;
        w.text(p.binding())?;
        for list in [p.skipped(), p.preferred()] {
            w.int(list.len() as i32)?;
            for t in list {
                w.int(*t as i32)?;
            }
        }
    }
    w.int(doc.layers().len() as i32)
}

/// レイヤーのパスを一覧の形（版 27 の属性のビット 6）で書くか: 塗りつぶしレイヤーのパス、2 本以上、名前を付けた・隠したパス、1 本のパスの
/// 並びで表せない設定（種類・筆先・深さ・対称・角・取っ手の点）を持つパス。
pub(crate) fn writes_path_list(layer: &yolu_core::Layer) -> bool {
    match layer.paths() {
        [] => false,
        // 塗りつぶしレイヤーのパスは、1 本の欄（ラスターレイヤーだけ）では書けない
        _ if layer.kind() == LayerKind::Fill => true,
        [one] => !one.name.is_empty() || !one.visible || needs_extra(&one.path),
        _ => true,
    }
}
/// 文書がパスの一覧の形で書くレイヤーを持つか（持っていれば正本の版は 27 になり、Unity 版は開けない）。
pub(crate) fn uses_path_lists(doc: &Document) -> bool {
    doc.layers().iter().any(writes_path_list)
}
/// 文書が Rust 版だけの Generator の種類（ノイズ・グランジ）の段を持つか（レイヤーの内容とマスクのスタック。無効な段も数える。
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
/// 文書が画像の Generator（種類 70）の段を持つか（レイヤーの内容とマスクのスタック。無効な段も数える）。持っていれば正本の版は 28 になり、
/// 版 25 までの読み手と Unity 版は開けない。
pub(crate) fn uses_image_generators(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        l.filters()
            .iter()
            .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
            .any(|e| {
                e.settings()
                    .generator_settings()
                    .is_some_and(|g| g.kind == generator::Kind::Image)
            })
    })
}
/// 版 29 の機能（塗りつぶしの点のグラデーションか、異方性のフィルターを切った塗りつぶしの画像）を使うレイヤーがあるか。
pub(crate) fn uses_point_gradient_version(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        l.kind() == LayerKind::Fill
            && (l.fill_point_gradients().next().is_some()
                || l.fill_images().any(|(c, _)| !l.fill_anisotropic(c)))
    })
}
/// 文書が、混色（Standard 以外のモード）か混合率曲線を使うグラデーションマップ（調整レイヤーか、レイヤーの内容・マスクのフィルターの段。無効な段も数える）を
/// 持つか。持っていれば正本の版は 25 になり、Unity 版は開けない。
pub(crate) fn uses_gradient_mixing(doc: &Document) -> bool {
    let mixes = |c: Option<ColorAdjust>| matches!(c, Some(ColorAdjust::GradientMap(g)) if g.ramp().uses_mixing());
    doc.layers().iter().any(|l| {
        l.adjustment().is_some_and(|a| mixes(a.color_adjust()))
            || l.filters()
                .iter()
                .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
                .any(|e| mixes(e.settings().color_adjust()))
    })
}
/// 文書が 0.5.0 の効果（レイヤーの内容とマスクのフィルターの段の種類 70〜79、Generator の種類 66〜69。無効な段も数える）を持つか。
/// 持っていれば正本の版は 28 になり、版 25 までの読み手と Unity 版は開けない。
pub(crate) fn uses_new_filters(doc: &Document) -> bool {
    doc.layers().iter().any(|l| {
        l.filters()
            .iter()
            .chain(l.mask().into_iter().flat_map(|m| m.filters().iter()))
            .any(|e| {
                (70..=79).contains(&e.settings().type_index())
                    || e.settings()
                        .generator_settings()
                        .is_some_and(|g| g.kind.is_050())
            })
    })
}
/// 文書が Rust 版だけの色調補正（調整レイヤーの種類 64〜69、レイヤーの内容とマスクのフィルターの段の種類 64〜69。無効な段も数える）を持つか。
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
/// 1 つのレイヤーを core に足す（C# の読み手と同じ順: 種類で作り、属性、チャンネルごとの合成、画素、マスク）。返すのは、読み終えてから
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
    // 続きの属性の印（ビット 7 のとき、ロックの直後）。ビット 0 は塗りつぶしの点のグラデーション
    let ext = if attributes & 128 != 0 {
        f.int(&format!("{p}.attributes_ext"))?
    } else {
        0
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
    // 塗りつぶしレイヤーの画素は、パスの一覧の画素（版 27 の属性のビット 6 のときだけ）
    let fill_paths = kind == 1 && attributes & 64 != 0;
    let channel_count = if kind == 0 || fill_paths {
        f.int(&format!("{p}.channel_count"))?
    } else {
        0
    };
    for k in 0..channel_count {
        let ch = format!("{p}.channels[{k}]");
        let c = f.channel(&format!("{ch}.channel"))?;
        if fill_paths {
            doc.ensure_fill_path_surface(id, c)?;
        }
        for t in 0..f.int(&format!("{ch}.tile_count"))? {
            let tile = format!("{ch}.tiles[{t}]");
            let coord = tile_coord(f, &tile)?;
            let bytes = f.bytes(&format!("{tile}.rgba"))?;
            if fill_paths {
                doc.import_fill_path_tile(id, c, coord, bytes)
            } else {
                doc.import_tile(id, c, coord, bytes)
            }
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
        let mut isotropic = Vec::new();
        for k in 0..n {
            let image = format!("{p}.images[{k}]");
            let channel = f.channel(&format!("{image}.channel"))?;
            images.push((
                channel,
                ImageId(core_id(f.guid(&format!("{image}.resource_id"))?)),
            ));
            if version >= POINT_GRADIENT_VERSION && !f.boolean(&format!("{image}.anisotropic"))? {
                isotropic.push(channel);
            }
        }
        let projection = read_projection(f, &format!("{p}.projection"))?;
        doc.set_fill_images_for_load(id, &images, projection)
            .map_err(|e| Error::from(e).in_context("塗りつぶしの画像・投影をcoreにできません"))?;
        if !isotropic.is_empty() {
            doc.set_fill_isotropic_for_load(id, &isotropic)
                .map_err(|e| {
                    Error::from(e).in_context("塗りつぶしの画像の読み方をcoreにできません")
                })?;
        }
    }
    if attributes & 32 != 0 {
        let mut gradients = Vec::new();
        for k in 0..f.int(&format!("{p}.gradient_count"))? {
            let g = format!("{p}.gradients[{k}]");
            gradients.push((f.channel(&format!("{g}.channel"))?, read_generator(f, &g)?));
        }
        doc.set_fill_gradients_for_load(id, gradients)
            .map_err(|e| {
                Error::from(e).in_context("塗りつぶしのグラデーションをcoreにできません")
            })?;
    }
    if ext & 1 != 0 {
        let mut list = Vec::new();
        for k in 0..f.int(&format!("{p}.point_gradient_count"))? {
            let g = format!("{p}.point_gradients[{k}]");
            let space = match f.int(&format!("{g}.space"))? {
                0 => PointSpace::Model,
                _ => PointSpace::Uv,
            };
            let mut points = Vec::new();
            for j in 0..f.int(&format!("{g}.point_count"))? {
                let q = format!("{g}.points[{j}]");
                points.push(GradientPoint {
                    position: [
                        f.float(&format!("{q}.x"))?,
                        f.float(&format!("{q}.y"))?,
                        f.float(&format!("{q}.z"))?,
                    ],
                    color: f.rgba(&format!("{q}.rgba"))?,
                });
            }
            list.push((
                f.channel(&format!("{g}.channel"))?,
                PointGradient {
                    space,
                    spread: f.float(&format!("{g}.spread"))?,
                    points,
                },
            ));
        }
        doc.set_fill_points_for_load(id, list).map_err(|e| {
            Error::from(e).in_context("塗りつぶしの点のグラデーションをcoreにできません")
        })?;
    }
    if version >= 8 && f.boolean(&format!("{p}.has_surface_path"))? {
        let path = erase_kind(read_path(f, &format!("{p}.surface_path"), true, version)?);
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
        let path = erase_kind(read_path(f, &format!("{p}.canvas_path"), false, version)?);
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
    if attributes & 64 != 0 {
        let mut entries = Vec::new();
        for k in 0..f.int(&format!("{p}.paths.count"))? {
            let item = format!("{p}.paths.items[{k}]");
            let mut path = read_path(
                f,
                &format!("{item}.path"),
                f.boolean(&format!("{item}.surface"))?,
                version,
            )?;
            read_path_extra(f, &format!("{item}.extra"), &mut path)?;
            entries.push(paths::LayerPathEntry {
                name: f.text(&format!("{item}.name"))?.to_owned(),
                visible: f.boolean(&format!("{item}.visible"))?,
                path,
            });
        }
        doc.set_paths_for_load(id, entries)
            .map_err(|e| Error::from(e).in_context("パスの一覧をcoreにできません"))?;
    }
    if ext & 2 != 0 {
        let text = read_text(f, &format!("{p}.text"))?;
        doc.set_text_for_load(id, text)
            .map_err(|e| Error::from(e).in_context("テキストの値をcoreにできません"))?;
    }
    if ext & 4 != 0 {
        let mut rulers = Vec::new();
        for k in 0..f.int(&format!("{p}.ruler_count"))? {
            rulers.push(read_ruler(f, &format!("{p}.rulers[{k}]"))?);
        }
        doc.set_rulers_for_load(id, rulers)
            .map_err(|e| Error::from(e).in_context("定規をcoreにできません"))?;
    }
    Ok(locks)
}

/// 定規 1 つ（`{p}` の下の項目。版 35）。
fn read_ruler(f: &Fields<'_>, p: &str) -> Result<Ruler> {
    let kind = RulerKind::from_index(f.byte(&format!("{p}.kind"))?)
        .ok_or_else(|| Error::InvalidData(format!("{p}.kind は範囲外です")))?;
    let scope = RulerScope::from_index(f.byte(&format!("{p}.scope"))?)
        .ok_or_else(|| Error::InvalidData(format!("{p}.scope は範囲外です")))?;
    let flags = f.byte(&format!("{p}.flags"))?;
    let point3 = |name: &str| -> Result<DVec3> {
        Ok(DVec3::new(
            f.float(&format!("{p}.{name}_x"))?,
            f.float(&format!("{p}.{name}_y"))?,
            f.float(&format!("{p}.{name}_z"))?,
        ))
    };
    let point2 = |name: &str| -> Result<DVec2> {
        Ok(DVec2::new(
            f.float(&format!("{p}.{name}_x"))?,
            f.float(&format!("{p}.{name}_y"))?,
        ))
    };
    let place = match f.byte(&format!("{p}.space"))? {
        0 => RulerPlace::Canvas {
            a: point2("a")?,
            b: point2("b")?,
        },
        _ => RulerPlace::Model {
            a: point3("a")?,
            b: point3("b")?,
            up: point3("up")?,
        },
    };
    Ok(Ruler {
        id: RulerId(core_id(f.guid(&format!("{p}.id"))?)),
        kind,
        place,
        two_points: flags & 4 != 0,
        lines: f.byte(&format!("{p}.lines"))?,
        line_symmetry: flags & 8 != 0,
        see_through: flags & 16 != 0,
        visible: flags & 1 != 0,
        scope,
        snap: flags & 2 != 0,
    })
}

/// 定規 1 つを書く（読み手の `rulers[i]` の並び。版 35）。
fn write_ruler(w: &mut Out<'_>, r: &Ruler) -> Result<()> {
    w.raw(&native_id(r.id.0))?;
    w.byte(r.kind.index())?;
    w.byte(match r.place {
        RulerPlace::Canvas { .. } => 0,
        RulerPlace::Model { .. } => 1,
    })?;
    // 印のビットは重ならない（読み手の `flags & …` と同じ並び）ので、立っているビットの値を足し合わせる
    let flags: u8 = [
        (r.visible, 1u8),
        (r.snap, 2),
        (r.two_points, 4),
        (r.line_symmetry, 8),
        (r.see_through, 16),
    ]
    .into_iter()
    .filter_map(|(on, bit)| on.then_some(bit))
    .sum();
    w.byte(flags)?;
    w.byte(r.scope.index())?;
    w.byte(r.lines)?;
    match r.place {
        RulerPlace::Canvas { a, b } => {
            for v in [a.x, a.y, b.x, b.y] {
                w.float(v)?;
            }
        }
        RulerPlace::Model { a, b, up } => {
            for v in [a.x, a.y, a.z, b.x, b.y, b.z, up.x, up.y, up.z] {
                w.float(v)?;
            }
        }
    }
    Ok(())
}

/// 1 本の欄（版 8・10・18）のパスの消しゴムの印を、消しゴムの種類にする（書き手は消しゴムの種類をこの印で書く。往復で同じ値）。
fn erase_kind(mut path: LayerPath) -> LayerPath {
    let (brush, style) = match &mut path {
        LayerPath::Canvas(c) => (&mut c.brush, &mut c.style),
        LayerPath::Surface(s) => (&mut s.brush, &mut s.style),
    };
    if brush.0.erase {
        brush.0.erase = false;
        style.kind = paths::PathKind::Erase;
    }
    path
}

/// テキストレイヤーの値（`{p}` の下の項目。版 30）。
fn read_text(f: &Fields<'_>, p: &str) -> Result<TextSettings> {
    let font = if f.int(&format!("{p}.font_kind"))? == 0 {
        TextFont::Bundled(f.text(&format!("{p}.font_name"))?.to_owned())
    } else {
        let hex = f.text(&format!("{p}.font_sha256"))?;
        let mut sha256 = [0u8; 32];
        for (i, b) in sha256.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
                .map_err(|_| Error::InvalidData(format!("{p}.font_sha256 が不正です")))?;
        }
        TextFont::File {
            path: f.text(&format!("{p}.font_path"))?.to_owned(),
            index: f.int(&format!("{p}.font_index"))? as u32,
            sha256,
            names: yolu_core::text::FontNames {
                family: f.text(&format!("{p}.font_family"))?.to_owned(),
                postscript: f.text(&format!("{p}.font_postscript"))?.to_owned(),
                weight: f.int(&format!("{p}.font_weight"))? as u16,
                italic: f.boolean(&format!("{p}.font_italic"))?,
            },
        }
    };
    let text = TextSettings {
        text: f.text(&format!("{p}.content"))?.to_owned(),
        font,
        size: f.float(&format!("{p}.size"))?,
        color: f.rgba(&format!("{p}.rgba"))?,
        line_height: f.float(&format!("{p}.line_height"))?,
        letter_spacing: f.float(&format!("{p}.letter_spacing"))?,
        align: match f.int(&format!("{p}.align"))? {
            0 => TextAlign::Left,
            1 => TextAlign::Center,
            _ => TextAlign::Right,
        },
        x: f.float(&format!("{p}.x"))?,
        y: f.float(&format!("{p}.y"))?,
        rotation: f.float(&format!("{p}.rotation"))?,
        wrap_width: f.float(&format!("{p}.wrap_width"))?,
    };
    text.validate()
        .map_err(|e| Error::from(e).in_context(format!("{p} を読めません")))?;
    Ok(text)
}

/// テキストレイヤーの値を書く（読み手の `text` の並び）。
fn write_text(w: &mut Out<'_>, text: &TextSettings) -> Result<()> {
    w.int(TEXT_ALGORITHM)?;
    w.text(&text.text)?;
    match &text.font {
        TextFont::Bundled(name) => {
            w.int(0)?;
            w.text(name)?;
        }
        TextFont::File {
            path,
            index,
            sha256,
            names,
        } => {
            w.int(1)?;
            w.text(path)?;
            w.int(
                i32::try_from(*index)
                    .map_err(|_| Error::InvalidData("フォントの束の番号が大きすぎます".into()))?,
            )?;
            let hex: String = sha256.iter().map(|b| format!("{b:02x}")).collect();
            w.text(&hex)?;
            w.text(&names.family)?;
            w.text(&names.postscript)?;
            w.int(i32::from(names.weight))?;
            w.boolean(names.italic)?;
        }
    }
    w.float(text.size)?;
    w.value(&text.color.to_array())?;
    w.float(text.line_height)?;
    w.float(text.letter_spacing)?;
    w.int(match text.align {
        TextAlign::Left => 0,
        TextAlign::Center => 1,
        TextAlign::Right => 2,
    })?;
    for v in [text.x, text.y, text.rotation, text.wrap_width] {
        w.float(v)?;
    }
    Ok(())
}

/// テキストレイヤーの並べ・塗りの版（正本の `text.algorithm`）。並べ・塗りの式を変えて同じ値の画素が変わるときに上げる（画素は保存してあるので、
/// 開いたときの見た目は変わらない。文を直したときの描き直しだけが新しい式になる）。
const TEXT_ALGORITHM: i32 = 1;

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
        // 版 34 から。前の版は なし（今の式）
        anti_alias: if version >= ANTI_ALIAS_VERSION {
            AntiAlias::from_index(f.byte(&format!("{p}.brush.anti_alias"))?).ok_or_else(|| {
                Error::InvalidData("パスのブラシのアンチエイリアスの段が不正です".into())
            })?
        } else {
            AntiAlias::None
        },
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
                tangent: paths::Tangent::Smooth,
            });
        }
        let path = paths::SurfacePath {
            style: Default::default(),
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
            style: Default::default(),
            id,
            channel,
            brush,
            points,
            material,
        })
    })
}

/// 一覧の 1 本の、1 本のパスの並びに無い設定（版 27 の `extra`）を、読んだパスへ当てる: 種類と、角・取っ手の点。
fn read_path_extra(f: &Fields<'_>, p: &str, path: &mut LayerPath) -> Result<()> {
    let kind = match f.byte(&format!("{p}.kind"))? {
        0 => paths::PathKind::Stroke,
        1 => paths::PathKind::Ribbon(paths::Ribbon {
            image: ImageId(core_id(f.guid(&format!("{p}.ribbon.image"))?)),
            mode: match f.byte(&format!("{p}.ribbon.mode"))? {
                0 => paths::RibbonMode::Tile,
                _ => paths::RibbonMode::Stretch,
            },
            spacing: f.float(&format!("{p}.ribbon.spacing"))?,
        }),
        2 => paths::PathKind::Fill,
        3 => paths::PathKind::Smudge {
            strength: f.float(&format!("{p}.strength"))?,
        },
        _ => paths::PathKind::Erase,
    };
    let tip = if f.boolean(&format!("{p}.has_tip"))? {
        let t = format!("{p}.tip");
        let w = f.int(&format!("{t}.width"))? as u32;
        let h = f.int(&format!("{t}.height"))? as u32;
        Some(std::sync::Arc::new(
            yolu_core::BrushTip::new(
                f.text(&format!("{t}.name"))?,
                w,
                h,
                f.bytes(&format!("{t}.alpha"))?.to_vec(),
            )
            .map_err(|e| Error::from(e).in_context("パスの筆先をcoreにできません"))?,
        ))
    } else {
        None
    };
    let style = paths::PathStyle {
        kind,
        tip,
        angle: f.float(&format!("{p}.angle"))?,
        follow: f.boolean(&format!("{p}.follow"))?,
        depth: if f.boolean(&format!("{p}.has_depth"))? {
            Some(f.float(&format!("{p}.depth"))?)
        } else {
            None
        },
        symmetry: match f.byte(&format!("{p}.symmetry"))? {
            1 => {
                let c = format!("{p}.canvas_symmetry");
                let mode = f.int(&format!("{c}.mode"))?;
                paths::PathSymmetry::Canvas(yolu_core::CanvasSymmetry {
                    mode: match mode {
                        1 => yolu_core::SymmetryMode::Vertical,
                        2 => yolu_core::SymmetryMode::Horizontal,
                        3 => yolu_core::SymmetryMode::Both,
                        5 => yolu_core::SymmetryMode::Lines,
                        _ => yolu_core::SymmetryMode::Radial,
                    },
                    center: yolu_core::glam::DVec2::new(
                        f.float(&format!("{c}.center_x"))?,
                        f.float(&format!("{c}.center_y"))?,
                    ),
                    count: f.int(&format!("{c}.count"))? as u32,
                    // 最初の線の角度（度）は種類 5（線対称）だけが持つ
                    angle: if mode == 5 {
                        f.float(&format!("{c}.angle"))?
                    } else {
                        0.0
                    },
                })
            }
            2 => {
                let m =
                    |k: &str| -> Result<f32> { Ok(f.float(&format!("{p}.mirror.{k}"))? as f32) };
                paths::PathSymmetry::Mirror {
                    point: yolu_core::glam::Vec3::new(m("point_x")?, m("point_y")?, m("point_z")?),
                    normal: yolu_core::glam::Vec3::new(
                        m("normal_x")?,
                        m("normal_y")?,
                        m("normal_z")?,
                    ),
                }
            }
            _ => paths::PathSymmetry::None,
        },
    };
    // 一覧の形では種類をそのまま持つ（1 本の欄の消しゴムの印は、消しゴムの種類のときだけ書くので外す）
    let erase = kind == paths::PathKind::Erase;
    match path {
        LayerPath::Canvas(c) => {
            c.style = style;
            if erase {
                c.brush.0.erase = false;
            }
        }
        LayerPath::Surface(s) => {
            s.style = style;
            if erase {
                s.brush.0.erase = false;
            }
        }
    }
    let n = f.int(&format!("{p}.tangent_count"))?;
    for i in 0..n {
        let t = format!("{p}.tangents[{i}]");
        let index = f.int(&format!("{t}.index"))? as usize;
        let kind = f.byte(&format!("{t}.kind"))?;
        let v = |side: &str, axis: &str| f.float(&format!("{t}.{side}.{axis}"));
        match path {
            LayerPath::Canvas(c) => {
                let point = c
                    .points
                    .get_mut(index)
                    .ok_or_else(|| Error::InvalidData("接線の点が無い".into()))?;
                point.tangent = match kind {
                    1 => paths::Tangent::Corner,
                    _ => paths::Tangent::Handles {
                        incoming: yolu_core::glam::DVec2::new(
                            v("incoming", "x")?,
                            v("incoming", "y")?,
                        ),
                        outgoing: yolu_core::glam::DVec2::new(
                            v("outgoing", "x")?,
                            v("outgoing", "y")?,
                        ),
                    },
                };
            }
            LayerPath::Surface(s) => {
                let point = s
                    .points
                    .get_mut(index)
                    .ok_or_else(|| Error::InvalidData("接線の点が無い".into()))?;
                let vec3 = |side: &str| -> Result<yolu_core::glam::Vec3> {
                    Ok(yolu_core::glam::Vec3::new(
                        v(side, "x")? as f32,
                        v(side, "y")? as f32,
                        v(side, "z")? as f32,
                    ))
                };
                point.tangent = match kind {
                    1 => paths::Tangent::Corner,
                    _ => paths::Tangent::Handles {
                        incoming: vec3("incoming")?,
                        outgoing: vec3("outgoing")?,
                    },
                };
            }
        }
    }
    Ok(())
}

/// 一覧の 1 本の拡張（版 27 の `extra`）を書く。
fn write_path_extra(w: &mut Out<'_>, path: &LayerPath) -> Result<()> {
    let style = match path {
        LayerPath::Canvas(c) => &c.style,
        LayerPath::Surface(s) => &s.style,
    };
    match style.kind {
        paths::PathKind::Stroke => w.byte(0)?,
        paths::PathKind::Ribbon(r) => {
            w.byte(1)?;
            w.raw(&native_id(r.image.0))?;
            w.byte(match r.mode {
                paths::RibbonMode::Tile => 0,
                paths::RibbonMode::Stretch => 1,
            })?;
            w.float(r.spacing)?;
        }
        paths::PathKind::Fill => w.byte(2)?,
        paths::PathKind::Smudge { strength } => {
            w.byte(3)?;
            w.float(strength)?;
        }
        paths::PathKind::Erase => w.byte(4)?,
    }
    w.boolean(style.tip.is_some())?;
    if let Some(tip) = &style.tip {
        w.text(tip.name())
            .map_err(|e| e.in_context("パスの筆先の名前"))?;
        w.int(tip.width() as i32)?;
        w.int(tip.height() as i32)?;
        w.value(tip.alpha())?;
    }
    w.float(style.angle)?;
    w.boolean(style.follow)?;
    w.boolean(style.depth.is_some())?;
    if let Some(d) = style.depth {
        w.float(d)?;
    }
    match style.symmetry {
        paths::PathSymmetry::None => w.byte(0)?,
        paths::PathSymmetry::Canvas(c) => {
            w.byte(1)?;
            check(
                w.rulers || c.mode != yolu_core::SymmetryMode::Lines,
                "パスの線対称は版 35 で書く",
            )?;
            w.int(match c.mode {
                yolu_core::SymmetryMode::Vertical => 1,
                yolu_core::SymmetryMode::Horizontal => 2,
                yolu_core::SymmetryMode::Both => 3,
                yolu_core::SymmetryMode::Lines => 5,
                // 対称なし（None）はパスの検査が断るので、ここへは来ない
                yolu_core::SymmetryMode::Radial | yolu_core::SymmetryMode::None => 4,
            })?;
            w.float(c.center.x)?;
            w.float(c.center.y)?;
            w.int(c.count as i32)?;
            if c.mode == yolu_core::SymmetryMode::Lines {
                w.float(c.angle)?;
            }
        }
        paths::PathSymmetry::Mirror { point, normal } => {
            w.byte(2)?;
            for v in [point.x, point.y, point.z, normal.x, normal.y, normal.z] {
                w.float(f64::from(v))?;
            }
        }
    }
    // (番号, 種類, 取っ手の成分)
    let tangents: Vec<(usize, u8, Vec<f64>)> = match path {
        LayerPath::Canvas(c) => c
            .points
            .iter()
            .enumerate()
            .filter_map(|(i, q)| match q.tangent {
                paths::Tangent::Smooth => None,
                paths::Tangent::Corner => Some((i, 1, Vec::new())),
                paths::Tangent::Handles { incoming, outgoing } => {
                    Some((i, 2, vec![incoming.x, incoming.y, outgoing.x, outgoing.y]))
                }
            })
            .collect(),
        LayerPath::Surface(s) => s
            .points
            .iter()
            .enumerate()
            .filter_map(|(i, q)| match q.tangent {
                paths::Tangent::Smooth => None,
                paths::Tangent::Corner => Some((i, 1, Vec::new())),
                paths::Tangent::Handles { incoming, outgoing } => Some((
                    i,
                    2,
                    [incoming.to_array(), outgoing.to_array()]
                        .concat()
                        .into_iter()
                        .map(f64::from)
                        .collect(),
                )),
            })
            .collect(),
    };
    w.int(tangents.len() as i32)?;
    for (i, kind, values) in tangents {
        w.int(i as i32)?;
        w.byte(kind)?;
        for v in values {
            w.float(v)?;
        }
    }
    Ok(())
}

/// パスが 1 本のパスの並び（版 8・10・18）で表せない設定を持つか（持っていれば一覧の形で書く）: 丸いブラシと消しゴム以外の種類、
/// 滑らかでない点。
fn needs_extra(path: &LayerPath) -> bool {
    match path {
        LayerPath::Canvas(c) => {
            !c.style.is_plain() || c.points.iter().any(|q| !q.tangent.is_smooth())
        }
        LayerPath::Surface(s) => {
            !s.style.is_plain() || s.points.iter().any(|q| !q.tangent.is_smooth())
        }
    }
}

/// 2D・3D のパス（C# の `WritePath`・`WriteCanvasPath`）。
fn write_path(w: &mut Out<'_>, path: &LayerPath) -> Result<()> {
    w.int(paths::ALGORITHM_VERSION as i32)?;
    w.raw(&native_id(path.id()))?;
    w.int(path.channel().index() as i32)?;
    let (mut brush, material, kind) = match path {
        LayerPath::Surface(p) => {
            w.text(&p.model_fingerprint)?;
            (p.brush.0, p.material.as_deref(), p.style.kind)
        }
        LayerPath::Canvas(p) => (p.brush.0, p.material.as_deref(), p.style.kind),
    };
    // 消しゴムの種類は、1 本の欄（Unity 版と同じ並び）ではブラシの消しゴムの印で表す
    brush.erase |= kind == paths::PathKind::Erase;
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
    if w.anti_alias {
        w.byte(brush.anti_alias.index())?;
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
        mode: ProjectionMode::try_from(
            u8::try_from(mode).map_err(|_| Error::InvalidData("投影の種類".into()))?,
        )
        .map_err(|e| Error::InvalidData(e.to_string()))?,
        wrap: Wrap::try_from(
            u8::try_from(wrap).map_err(|_| Error::InvalidData("投影の外側".into()))?,
        )
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
        66 => generator::Kind::Pattern,
        68 => generator::Kind::Light,
        69 => generator::Kind::MaskBuilder,
        70 => generator::Kind::Image,
        67 => generator::Kind::UvIslandVariation,
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
    g.axis = usize::try_from(f.int(&format!("{p}.axis"))?)
        .map_err(|_| Error::InvalidData("軸".into()))?;
    g.direction = [v("direction_x")?, v("direction_y")?, v("direction_z")?];
    g.use_bent_normal = f.boolean(&format!("{p}.bent_normal"))?;
    for k in 0..f.int(&format!("{p}.pin_count"))? {
        let pin = format!("{p}.pins[{k}]");
        let map = f.int(&format!("{pin}.kind"))?;
        let map = map_kind(map)
            .ok_or_else(|| Error::InvalidData(format!("{pin}.kind {map} は範囲外です")))?;
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
    if kind == generator::Kind::Image {
        // 画像の欄（正本の版 28。`write_generator` の末尾と対）
        let q = format!("{p}.effect");
        let component = f.int(&format!("{q}.component"))?;
        g.image = generator::ImageSource {
            image: core_id(f.guid(&format!("{q}.resource_id"))?),
            projection: read_projection(f, &format!("{q}.projection"))?,
            component: generator::ImageComponent::from_index(i64::from(component)).ok_or_else(
                || Error::InvalidData(format!("{q}.component {component} は範囲外です")),
            )?,
        };
    }
    if kind.is_050() {
        read_generator_effect(f, &format!("{p}.effect"), &mut g)?;
    }
    Ok(g)
}

/// 模様・ライト・マスクの組み立て・アイランドごとのばらつきの欄（正本の版 28。`write_generator_effect` と対）。
fn read_generator_effect(f: &Fields<'_>, p: &str, g: &mut generator::Settings) -> Result<()> {
    let int = |name: &str| f.int(&format!("{p}.{name}"));
    let float = |name: &str| f.float(&format!("{p}.{name}"));
    let bad = |name: &str| Error::InvalidData(format!("{p}.{name} は範囲外です"));
    match g.kind {
        generator::Kind::Pattern => {
            g.pattern = generator::Pattern {
                shape: generator::PatternShape::from_index(i64::from(int("shape")?))
                    .ok_or_else(|| bad("shape"))?,
                scale: float("scale")?,
                angle: float("angle")?,
                width: float("width")?,
                softness: float("softness")?,
                offset: [float("offset_u")?, float("offset_v")?],
            };
        }
        generator::Kind::Light => {
            g.light = generator::Light {
                azimuth: float("azimuth")?,
                elevation: float("elevation")?,
                softness: float("softness")?,
                ambient: float("ambient")?,
            };
        }
        generator::Kind::UvIslandVariation => {
            g.island = generator::IslandVariation {
                seed: int("seed")?,
                min: float("min")?,
                max: float("max")?,
            };
        }
        _ => {
            for (map, input) in ["curvature", "ambient_occlusion", "position", "thickness"]
                .iter()
                .zip(g.mask_builder.inputs.iter_mut())
            {
                *input = generator::MaskInput {
                    weight: float(&format!("{map}.weight"))?,
                    level: float(&format!("{map}.level"))?,
                    contrast: float(&format!("{map}.contrast"))?,
                    invert: f.boolean(&format!("{p}.{map}.invert"))?,
                };
            }
            g.mask_builder.combine = generator::MaskCombine::from_index(i64::from(int("combine")?))
                .ok_or_else(|| bad("combine"))?;
        }
    }
    Ok(())
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
/// 版 28 のフィルターの段（種類 70〜79。`effect` の塊。`write_effect_filter` と対）。
fn read_effect_filter(f: &Fields<'_>, p: &str, kind: i32) -> Result<EffectSettings> {
    use yolu_core::filter::{MorphologyMode, Settings as F, SlopeMode};
    let float = |name: &str| f.float(&format!("{p}.{name}"));
    let int = |name: &str| f.int(&format!("{p}.{name}"));
    let uint = |name: &str| -> Result<u32> {
        u32::try_from(int(name)?).map_err(|_| Error::InvalidData(format!("{p}.{name} が負です")))
    };
    Ok(EffectSettings::Filter(match kind {
        70 => F::HistogramScan {
            position: float("position")?,
            contrast: float("contrast")?,
        },
        71 => F::HistogramRange {
            range: float("range")?,
            position: float("position")?,
        },
        72 => F::SlopeBlur {
            intensity: float("intensity")?,
            samples: uint("samples")?,
            mode: match int("mode")? {
                1 => SlopeMode::Min,
                2 => SlopeMode::Max,
                _ => SlopeMode::Blur,
            },
            scale: float("scale")?,
            seed: int("seed")?,
        },
        73 => F::DirectionalBlur {
            angle: float("angle")?,
            distance: float("distance")?,
        },
        74 => F::Warp {
            intensity: float("intensity")?,
            scale: float("scale")?,
            seed: int("seed")?,
        },
        75 => F::Morphology {
            mode: if int("mode")? == 1 {
                MorphologyMode::Erode
            } else {
                MorphologyMode::Dilate
            },
            radius: uint("radius")?,
        },
        76 => F::EdgeDetect {
            width: uint("width")?,
            threshold: float("threshold")?,
        },
        77 => F::HighPass {
            radius: uint("radius")?,
        },
        78 => F::Median {
            radius: uint("radius")?,
        },
        _ => F::Glow {
            threshold: float("threshold")?,
            radius: uint("radius")?,
            intensity: float("intensity")?,
        },
    }))
}

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
            t @ 70..=79 => read_effect_filter(f, &format!("{item}.effect"), t)?,
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
    /// 版 29 の並び（画像ごとの異方性・点のグラデーション）で書くか。
    points: bool,
    /// 版 30 の文字の値を書けるか。
    text: bool,
    /// 版 34 のパスのブラシのアンチエイリアスの段を書くか。
    anti_alias: bool,
    /// 版 35 の定規と、パスの線対称（種類 5）を書けるか。
    rulers: bool,
}
impl<'s> Out<'s> {
    /// 正本の版 `version` の並びで書く。
    fn for_version(sink: &'s mut dyn Sink, version: i32) -> Out<'s> {
        Out {
            sink,
            mixing: version >= MIXING_VERSION,
            points: version >= POINT_GRADIENT_VERSION,
            text: version >= TEXT_VERSION,
            anti_alias: version >= ANTI_ALIAS_VERSION,
            rulers: version >= RULERS_VERSION,
        }
    }
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

/// 1 つのレイヤー（C# の `DocumentBinary.Write` のレイヤーの並び）。
fn write_layer(w: &mut Out<'_>, layer: &yolu_core::Layer) -> Result<()> {
    let named = |why: &str| format!("レイヤー「{}」の{why}", layer.name());
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
    let list = writes_path_list(layer);
    let points: Vec<(Channel, &PointGradient)> = layer.fill_point_gradients().collect();
    // 版の決め（`version_of`）が、点のグラデーション・異方性を切った画像のある文書を版 29 にする。ここに来て古い版なら書けない
    check(
        w.points || points.is_empty() && images.iter().all(|(c, _)| layer.fill_anisotropic(*c)),
        named("点のグラデーション・画像の読み方は版 29 で書く"),
    )?;
    // 属性の印: ビット 0 クリッピング、ビット 1 ロックが続く、ビット 2 チャンネルごとの設定が続く、ビット 3 塗りつぶしの画像、ビット 4 Anchor、
    // ビット 5 塗りつぶしのグラデーション、ビット 6 パスの一覧（版 27）、ビット 7 続きの属性の印が続く（版 29）。ロックの印（int、0 は書かない）は
    // 属性の直後、続きの属性の印（int、0 は書かない。ビット 0 塗りつぶしの点のグラデーション、ビット 1 文字の値（版 30）、ビット 2 定規（版 35））はロックの直後、
    // どちらもチャンネルごとの設定より前
    check(
        w.text || layer.text().is_none(),
        named("テキストレイヤーは版 30 で書く"),
    )?;
    check(
        w.rulers || layer.rulers().is_empty(),
        named("定規は版 35 で書く"),
    )?;
    let ext: i32 = if points.is_empty() { 0 } else { 1 }
        | if layer.text().is_some() { 2 } else { 0 }
        | if layer.rulers().is_empty() { 0 } else { 4 };
    w.byte(
        u8::from(layer.clipping())
            | if locks == LayerLocks::NONE { 0 } else { 2 }
            | if blends.is_empty() { 0 } else { 4 }
            | if fill_images { 8 } else { 0 }
            | if anchors { 16 } else { 0 }
            | if gradients.is_empty() { 0 } else { 32 }
            | if list { 64 } else { 0 }
            | if ext == 0 { 0 } else { 128 },
    )?;
    if locks != LayerLocks::NONE {
        w.int(i32::from(locks.bits()))?;
    }
    if ext != 0 {
        w.int(ext)?;
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
            if w.points {
                w.boolean(layer.fill_anisotropic(*c))?;
            }
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
    if !points.is_empty() {
        w.int(points.len() as i32)?;
        for (c, g) in &points {
            w.int(c.index() as i32)?;
            w.int(1)?; // アルゴリズムの版（逆距離の 2 乗の重みと広がり）
            w.int(g.space as i32)?;
            w.float(g.spread)?;
            w.int(g.points.len() as i32)?;
            for p in &g.points {
                for v in p.position {
                    w.float(v)?;
                }
                w.value(&p.color.to_array())?;
            }
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
    if layer.kind() == LayerKind::Fill && list {
        // 塗りつぶしレイヤーのパスの画素（パスの一覧と一緒に読む）
    } else if layer.kind() == LayerKind::Raster {
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
            named("画素はラスターレイヤーだけが持てます"),
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
    let surface_path = layer.path().filter(|p| !list && !p.is_canvas());
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
    let canvas_path = layer.path().filter(|p| !list && p.is_canvas());
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
    if list {
        w.int(layer.paths().len() as i32)?;
        for e in layer.paths() {
            w.text(&e.name)
                .map_err(|err| err.in_context(named("パスの名前")))?;
            w.boolean(e.visible)?;
            w.boolean(!e.path.is_canvas())?;
            write_path(w, &e.path)?;
            write_path_extra(w, &e.path)?;
        }
    }
    if let Some(text) = layer.text() {
        write_text(w, text)?;
    }
    if !layer.rulers().is_empty() {
        w.int(layer.rulers().len() as i32)?;
        for r in layer.rulers() {
            write_ruler(w, r)?;
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
    if g.kind == generator::Kind::Image {
        // 画像の欄（正本の版 28。`read_generator` と対）: 画像の ID（選んでいなければ空）・投影・成分
        let image = &g.image;
        w.raw(&native_id(image.image))?;
        write_projection(w, &image.projection)?;
        w.int(image.component as i32)?;
    }
    if g.kind.is_050() {
        write_generator_effect(w, g)?;
    }
    Ok(())
}
/// 模様・ライト・マスクの組み立て・アイランドごとのばらつきの欄（正本の版 28。`read_generator_effect` と対）。
fn write_generator_effect(w: &mut Out<'_>, g: &generator::Settings) -> Result<()> {
    match g.kind {
        generator::Kind::Pattern => {
            let p = &g.pattern;
            w.int(p.shape as i32)?;
            for v in [
                p.scale,
                p.angle,
                p.width,
                p.softness,
                p.offset[0],
                p.offset[1],
            ] {
                w.float(v)?;
            }
        }
        generator::Kind::Light => {
            let l = &g.light;
            for v in [l.azimuth, l.elevation, l.softness, l.ambient] {
                w.float(v)?;
            }
        }
        generator::Kind::UvIslandVariation => {
            let v = &g.island;
            w.int(v.seed)?;
            w.float(v.min)?;
            w.float(v.max)?;
        }
        _ => {
            for input in &g.mask_builder.inputs {
                w.float(input.weight)?;
                w.float(input.level)?;
                w.float(input.contrast)?;
                w.boolean(input.invert)?;
            }
            w.int(g.mask_builder.combine as i32)?;
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
        if let EffectSettings::Filter(f) = e.settings() {
            write_effect_filter(w, f)?;
        }
    }
    Ok(())
}
/// 版 28 のフィルターの段（種類 70〜79）の `effect` の塊（`read_effect_filter` と対）。ほかの種類は何も書かない。
fn write_effect_filter(w: &mut Out<'_>, f: &yolu_core::filter::Settings) -> Result<()> {
    use yolu_core::filter::{MorphologyMode, Settings as F, SlopeMode};
    match *f {
        F::HistogramScan { position, contrast } => {
            w.float(position)?;
            w.float(contrast)?;
        }
        F::HistogramRange { range, position } => {
            w.float(range)?;
            w.float(position)?;
        }
        F::SlopeBlur {
            intensity,
            samples,
            mode,
            scale,
            seed,
        } => {
            w.float(intensity)?;
            w.int(samples as i32)?;
            w.int(match mode {
                SlopeMode::Blur => 0,
                SlopeMode::Min => 1,
                SlopeMode::Max => 2,
            })?;
            w.float(scale)?;
            w.int(seed)?;
        }
        F::DirectionalBlur { angle, distance } => {
            w.float(angle)?;
            w.float(distance)?;
        }
        F::Warp {
            intensity,
            scale,
            seed,
        } => {
            w.float(intensity)?;
            w.float(scale)?;
            w.int(seed)?;
        }
        F::Morphology { mode, radius } => {
            w.int(i32::from(mode == MorphologyMode::Erode))?;
            w.int(radius as i32)?;
        }
        F::EdgeDetect { width, threshold } => {
            w.int(width as i32)?;
            w.float(threshold)?;
        }
        F::HighPass { radius } | F::Median { radius } => w.int(radius as i32)?,
        F::Glow {
            threshold,
            radius,
            intensity,
        } => {
            w.float(threshold)?;
            w.int(radius as i32)?;
            w.float(intensity)?;
        }
        _ => {}
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
