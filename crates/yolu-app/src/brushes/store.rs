//! 利用者のブラシの保存（設定のフォルダの `brushes/`）。ブラシ 1 つが 1 ファイル（`brush-<番号>.ylbrush`）、並びは `order.conf`。
//!
//! 形式は 1 行目が `yolupainter-brush 1`（筆圧の応えを使うブラシだけ `yolupainter-brush 2`。最小値・曲線に加え、硬さを筆圧で変える切り替えだけでも 2 になる。
//! 色の混ぜ（厚塗り）を使うブラシだけ `yolupainter-brush 3`（筆圧の応えも使えば、その項目も同じファイルに書く）。
//! 入り抜き・手ぶれ補正を持つブラシ（取り込んだブラシが持つ分）だけ `yolupainter-brush 4`（`assist.*`。ほかの版の項目も同じファイルに書く）。
//! 縁のアンチエイリアスが なし でないブラシだけ `yolupainter-brush 5`（`anti_alias`。ほかの版の項目も同じファイルに書く）。
//! 版 2〜5 を知らない古いアプリは、そのファイルを「新しい形式」として触らずに読み飛ばす）、あとは `key=value` の行（UTF-8、64 KiB まで）。数は Rust の表記のまま書き（読み戻しても
//! 同じ値）、筆先・質感の画像は札（トークン）で指す: 組み込みの名前（`grain`）、同梱の Krita の筆先の ID（`bundled:krita4/<ファイル>`）、
//! 取り込んだ画像（`img:<SHA-256>`。画像は `images/<SHA-256>.png` に 1 枚ずつ置く。`images.rs`）。取り込んだブラシには、出どころと
//! 表せなかった項目の印（`import.*`）が付く。知らない項目・重なった項目・範囲を外れた値は
//! そのファイルを読み飛ばして理由を残す（ほかのファイルは読む）。版が新しいファイルは触らずに読み飛ばす。
//! 書き込みは、画像を先に置き（一時ファイルへ書いて読み戻して確かめてから置換）、ブラシのファイルを最後の 1 回の置換で確定する
//! （途中で落ちても、ブラシのファイルは前の版のままか新しい版のどちらか。置いただけの画像は、内容で名づけるので無害）。
//! 名前はファイルの中だけに持ち、ファイル名には使わない。

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::gaps::Gap;
use super::images;
use super::{canonical, carried_assist, Group, ImportMeta, UserBrush, MAX_NAME_CHARS};
use crate::engine::{
    AntiAlias, Brush, BrushEffect, ColorMix, CoreError, DVec2, DualBrush, DualBrushMode, MixGround,
    MixMode, PaperTexture, PressureResponse, StrokeAssist, TextureMode,
};
use crate::lang::Lang;
use yolu_core::brush::{builtin_tip, TipSelection, MAX_CURVE_POINTS};
use yolu_core::generator::CurvePoint;
use yolu_core::BrushTip;
use yolu_io::brushes::bundled;

pub const HEADER: &str = "yolupainter-brush 1";
/// 筆圧の応え（最小値・曲線・硬さの切り替え）を使うブラシの版。使わないブラシは版 1 のままで、今までと同じバイト。版 1 しか読めない
/// 古いアプリは、このファイルを「新しい形式」として理由つきで読み飛ばす。
pub const HEADER_V2: &str = "yolupainter-brush 2";
/// 色の混ぜ（厚塗り）を使うブラシの版。使わないブラシは版 1・2 のままで、今までと同じバイト。版 2 までしか読めない古いアプリは、
/// このファイルを「新しい形式」として理由つきで読み飛ばす。
pub const HEADER_V3: &str = "yolupainter-brush 3";
/// 入り抜き・手ぶれ補正を持つブラシの版（`assist.stabilizer`・`assist.taper_in`・`assist.taper_out`）。使わないブラシは版 1〜3 のままで、
/// 今までと同じバイト。版 3 までしか読めない古いアプリは、このファイルを「新しい形式」として理由つきで読み飛ばす。
pub const HEADER_V4: &str = "yolupainter-brush 4";
/// 縁のアンチエイリアス（`anti_alias`）が なし でないブラシの版。なし のブラシは版 1〜4 のままで、今までと同じバイト（読むときも、
/// 版 4 までのファイルは なし）。版 4 までしか読めない古いアプリは、このファイルを「新しい形式」として理由つきで読み飛ばす。
pub const HEADER_V5: &str = "yolupainter-brush 5";
const EXTENSION: &str = "ylbrush";
const ORDER_FILE: &str = "order.conf";
/// 1 ファイルの大きさの上限。
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// 読むファイルの数の上限（これより多い分は読み飛ばす）。
const MAX_FILES: usize = 1024;
/// 取り込んだブラシの出どころの名前の長さの上限（文字数）。
pub const MAX_SOURCE_CHARS: usize = 64;

/// 読み書きの失敗の種類（言語ごとの短い理由は `describe`）。
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    TooLarge,
    /// 1 行目がこの形式でない。
    NotABrush,
    /// 今の版より新しい形式。
    NewerVersion(String),
    /// `key=value` でない行（行番号）。
    Syntax(usize),
    UnknownKey(String),
    DuplicateKey(String),
    BadValue(String),
    /// 組み込みにない筆先・質感の名前。
    UnknownTip(String),
    /// core の検証が断った設定。
    Invalid(CoreError),
    /// 画像のファイルが読めない・壊れている・名前（指紋）と中身が合わない（先頭の数文字）。
    BadImage(String),
    /// 書いたファイルを読み戻したら、書いた設定と違った。
    Mismatch,
    TooMany,
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl StoreError {
    pub fn describe(&self, lang: Lang) -> String {
        match self {
            StoreError::Io(e) => lang.file_error(e),
            StoreError::TooLarge => lang
                .pick("ファイルが大きすぎます", "The file is too large")
                .into(),
            StoreError::NotABrush => lang
                .pick("ブラシのファイルではありません", "Not a brush file")
                .into(),
            StoreError::NewerVersion(v) => lang.pick(
                format!("新しい形式です（{v}）"),
                format!("A newer format ({v})"),
            ),
            StoreError::Syntax(line) => lang.pick(
                format!("{line} 行目が読めません"),
                format!("Cannot read line {line}"),
            ),
            StoreError::UnknownKey(k) => lang.pick(
                format!("知らない項目「{k}」があります"),
                format!("Unknown item \"{k}\""),
            ),
            StoreError::DuplicateKey(k) => lang.pick(
                format!("項目「{k}」が重なっています"),
                format!("Repeated item \"{k}\""),
            ),
            StoreError::BadValue(k) => lang.pick(
                format!("項目「{k}」の値が読めません"),
                format!("Invalid value of \"{k}\""),
            ),
            StoreError::UnknownTip(n) => lang.pick(
                format!("知らない画像「{n}」があります"),
                format!("Unknown image \"{n}\""),
            ),
            StoreError::Invalid(e) => lang.core_error(e),
            StoreError::BadImage(short) => lang.pick(
                format!("画像「{short}」を読めません"),
                format!("Cannot read the image \"{short}\""),
            ),
            StoreError::Mismatch => lang
                .pick(
                    "書いた内容を読み戻せませんでした",
                    "The written file did not read back the same",
                )
                .into(),
            StoreError::TooMany => lang.pick("ファイルが多すぎます", "Too many files").into(),
        }
    }
}

/// 読めなかったファイルと、その理由。
#[derive(Debug)]
pub struct Problem {
    pub file: String,
    pub reason: StoreError,
}

impl Problem {
    pub fn describe(&self, lang: Lang) -> String {
        self.reason.describe(lang)
    }
}

/// フォルダを読んだ結果。
#[derive(Debug, Default)]
pub struct LoadReport {
    pub brushes: Vec<UserBrush>,
    /// フォルダにあるブラシのファイル名（`brush-<番号>.ylbrush`）の番号の最大。読めなかったもの・数の上限で読まなかったもの・
    /// ファイルでないものも含む（次に付ける番号が、それらの番号に当たって置き換えてしまわないように）。
    pub max_file_id: Option<u32>,
    /// フォルダにあるのに読み込まなかったブラシのファイルの番号（読めなかった物・数の上限で読まなかった物）。ツールの並びが、
    /// この番号の札を消さずに覚えておくために使う（フォルダに無い番号の札は覚えない）。
    pub unloaded: Vec<u32>,
    /// 並びの札（`BrushKey::token`）。
    pub order: Vec<String>,
    pub problems: Vec<Problem>,
}

// ───────── 文字の形式 ─────────

fn mode_id(mode: TextureMode) -> &'static str {
    match mode {
        TextureMode::Multiply => "multiply",
        TextureMode::Subtract => "subtract",
        TextureMode::Darken => "darken",
        TextureMode::Overlay => "overlay",
        TextureMode::ColorDodge => "color-dodge",
        TextureMode::ColorBurn => "color-burn",
        TextureMode::LinearBurn => "linear-burn",
        TextureMode::HardMix => "hard-mix",
    }
}

fn dual_mode_id(mode: DualBrushMode) -> &'static str {
    match mode {
        DualBrushMode::Multiply => "multiply",
        DualBrushMode::Darken => "darken",
        DualBrushMode::Overlay => "overlay",
        DualBrushMode::ColorDodge => "color-dodge",
        DualBrushMode::ColorBurn => "color-burn",
        DualBrushMode::LinearBurn => "linear-burn",
        DualBrushMode::HardMix => "hard-mix",
        DualBrushMode::Subtract => "subtract",
    }
}

/// 同梱の Krita の筆先を読み込み済みか（読み込む前は、書くときに同梱の筆先かどうかを調べない）。同梱の筆先は読み込みが済んでから
/// でないとブラシの中にいない（`img:` で置いても内容は同じ）ので、書くために 100 ms 級の読み込みを画面の側で待たない。
static KRITA_LOADED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 同梱の Krita の筆先（初めて呼ぶと全部を読むので、画面のスレッドでは呼ばない）。呼んだことを覚える。
pub fn krita() -> &'static bundled::BundledSet {
    let set = bundled::krita4();
    KRITA_LOADED.store(true, std::sync::atomic::Ordering::Release);
    set
}

/// 読み込み済みなら同梱の Krita の筆先（読み込みを待たない）。
pub fn krita_loaded() -> Option<&'static bundled::BundledSet> {
    KRITA_LOADED
        .load(std::sync::atomic::Ordering::Acquire)
        .then(krita)
}

const IMAGE_PREFIX: &str = "img:";

/// 書き出すブラシが指す取り込んだ画像（指紋と画像。重ならない）。
#[derive(Default)]
struct Pending {
    images: Vec<(String, Arc<BrushTip>)>,
}

impl Pending {
    /// 画像 1 枚の札: 組み込みの名前（名前だけ同じで中身が違う画像は組み込みとして扱わない）、同梱の Krita の筆先の ID、
    /// そうでなければ取り込んだ画像（`img:<指紋>`）。
    fn token(&mut self, tip: &Arc<BrushTip>) -> String {
        if let Some(b) = builtin_tip(tip.name()).filter(|b| **b == **tip) {
            return b.name().to_owned();
        }
        if let Some(set) = krita_loaded() {
            let found = set
                .brushes
                .iter()
                .find(|b| b.brush.tip.image.as_deref() == Some(&**tip));
            if let Some(b) = found {
                return b.id.clone();
            }
        }
        let hash = images::fingerprint(tip);
        if !self.images.iter().any(|(h, _)| *h == hash) {
            self.images.push((hash.clone(), tip.clone()));
        }
        format!("{IMAGE_PREFIX}{hash}")
    }

    /// 複数の筆先（ホース）の札: 同梱の Krita のホースと同じなら ID 1 つ、そうでなければ札の並び。
    fn list(&mut self, tips: &[Arc<BrushTip>]) -> String {
        if let Some(set) = krita_loaded() {
            let found = set
                .brushes
                .iter()
                .find(|b| !b.brush.tip.images.is_empty() && b.brush.tip.images == tips);
            if let Some(b) = found {
                return b.id.clone();
            }
        }
        let tokens: Vec<String> = tips.iter().map(|t| self.token(t)).collect();
        tokens.join(",")
    }
}

/// 札から画像の並びへ（同梱のホースは複数、それ以外は 1 枚）。`load` は取り込んだ画像（指紋）を読む。
fn resolve(
    token: &str,
    load: &mut dyn FnMut(&str) -> Result<Arc<BrushTip>, StoreError>,
) -> Result<Vec<Arc<BrushTip>>, StoreError> {
    if let Some(hash) = token.strip_prefix(IMAGE_PREFIX) {
        if !images::is_fingerprint(hash) {
            return Err(StoreError::UnknownTip(token.chars().take(32).collect()));
        }
        return Ok(vec![load(hash)?]);
    }
    if token.starts_with(bundled::ID_PREFIX) {
        let b = krita()
            .brushes
            .iter()
            .find(|b| b.id == token)
            .ok_or_else(|| StoreError::UnknownTip(token.chars().take(64).collect()))?;
        return Ok(match &b.brush.tip.image {
            Some(image) => vec![image.clone()],
            None => b.brush.tip.images.clone(),
        });
    }
    builtin_tip(token)
        .map(|t| vec![t])
        .ok_or_else(|| StoreError::UnknownTip(token.chars().take(64).collect()))
}

struct Writer(String);

impl Writer {
    fn line(&mut self, key: &str, value: impl std::fmt::Display) {
        self.0.push_str(&format!("{key}={value}\n"));
    }
    fn bool(&mut self, key: &str, value: bool) {
        self.line(key, value as u8);
    }
    /// 画面の精度（f32）で持つ値。
    fn single(&mut self, key: &str, value: f64) {
        self.line(key, value as f32);
    }
}

/// 書き出した結果: ブラシの文と、その文が指す取り込んだ画像（指紋と画像。先に置いてから文を置く）。
#[derive(Debug)]
pub struct Encoded {
    pub text: String,
    pub images: Vec<(String, Arc<BrushTip>)>,
}

/// ブラシを書き出す（`brush` は正規の形でなくても、画面が持たない項目は書かない）。
pub fn encode(user: &UserBrush) -> Result<Encoded, StoreError> {
    let b = canonical(&user.brush);
    let assist = carried_assist(user.assist);
    let mut pending = Pending::default();
    let header = if b.base.anti_alias != AntiAlias::None {
        HEADER_V5
    } else if assist.is_some() {
        HEADER_V4
    } else if uses_mix(&b) {
        HEADER_V3
    } else if uses_pressure_response(&b) {
        HEADER_V2
    } else {
        HEADER
    };
    let mut w = Writer(format!("{header}\n"));
    let name: String = user.name.chars().take(MAX_NAME_CHARS).collect();
    w.line("name", name.replace(['\n', '\r'], " "));
    w.line("group", user.group.id());
    w.single("radius", b.base.radius);
    w.single("hardness", b.base.hardness);
    w.single("spacing", b.base.spacing);
    w.single("opacity", b.base.opacity);
    w.single("flow", b.base.flow);
    w.bool("pressure_size", b.base.pressure_size);
    w.bool("pressure_opacity", b.base.pressure_opacity);
    w.bool("pressure_flow", b.base.pressure_flow);
    // 縁のアンチエイリアス（版 5。なし のブラシは書かない）
    if b.base.anti_alias != AntiAlias::None {
        w.line("anti_alias", b.base.anti_alias.id());
    }
    let t = &b.tip;
    match &t.image {
        Some(image) => w.line("tip.image", pending.token(image)),
        None => w.line("tip.image", "none"),
    }
    if t.images.is_empty() {
        w.line("tip.images", "none");
    } else {
        w.line("tip.images", pending.list(&t.images));
    }
    w.line(
        "tip.selection",
        match t.selection {
            TipSelection::Random => "random",
            TipSelection::Sequential => "sequential",
        },
    );
    w.line("tip.angle", t.angle);
    w.line("tip.roundness", t.roundness);
    w.bool("tip.follow", t.follow_direction);
    w.bool("tip.flip_x", t.flip_x);
    w.bool("tip.flip_y", t.flip_y);
    let j = &b.jitter;
    w.line("jitter.size", j.size);
    w.line("jitter.angle", j.angle);
    w.line("jitter.roundness", j.roundness);
    w.line("jitter.opacity", j.opacity);
    w.line("jitter.flow", j.flow);
    w.line("jitter.scatter", j.scatter);
    w.line("jitter.count", j.count);
    if let Some(tex) = &b.texture {
        w.line("texture", pending.token(&tex.image));
        w.line("texture.depth", tex.depth);
        w.line("texture.scale", tex.scale);
        w.line("texture.mode", mode_id(tex.mode));
    }
    if let Some(d) = &b.dual {
        w.line("dual", 1);
        match &d.tip {
            Some(image) => w.line("dual.tip", pending.token(image)),
            None => w.line("dual.tip", "none"),
        }
        w.line("dual.radius", d.radius);
        w.line("dual.hardness", d.hardness);
        w.line("dual.spacing", d.spacing);
        w.line("dual.angle", d.angle);
        w.line("dual.roundness", d.roundness);
        w.line("dual.scatter", d.scatter);
        w.line("dual.count", d.count);
        w.line("dual.mode", dual_mode_id(d.mode));
    }
    let c = &b.color;
    w.line("color.fg_bg", c.foreground_background);
    w.line("color.hue", c.hue);
    w.line("color.saturation", c.saturation);
    w.line("color.brightness", c.brightness);
    w.line("color.purity", c.purity);
    w.bool("color.per_tip", c.per_tip);
    let k = &b.controls;
    w.line("controls.fade_size", k.fade_size);
    w.line("controls.fade_opacity", k.fade_opacity);
    w.line("controls.fade_flow", k.fade_flow);
    w.bool("controls.tilt_size", k.tilt_size);
    w.bool("controls.tilt_opacity", k.tilt_opacity);
    w.bool("controls.tilt_flow", k.tilt_flow);
    w.bool("controls.tilt_angle", k.tilt_angle);
    w.bool("controls.rotation_angle", k.rotation_angle);
    w.bool("controls.speed_size", k.speed_size);
    w.bool("controls.speed_opacity", k.speed_opacity);
    w.bool("controls.speed_flow", k.speed_flow);
    w.line("controls.speed_max", k.speed_max);
    // 筆圧の応え（版 2。既定の項目は書かない）
    if k.pressure_hardness {
        w.bool("controls.pressure_hardness", true);
    }
    for (name, r) in pressure_items(&b) {
        if r.min() != 0.0 {
            w.single(&format!("pressure.{name}.min"), r.min());
        }
        if !r.curve().is_empty() {
            w.line(&format!("pressure.{name}.curve"), curve_text(r.curve()));
        }
    }
    // 色の混ぜ（版 3。混ぜ方が切のブラシは書かない）
    if uses_mix(&b) {
        let m = &b.mix;
        w.line("mix.mode", mix_mode_id(m.mode));
        w.single("mix.paint", m.paint);
        w.single("mix.density", m.density);
        w.single("mix.stretch", m.stretch);
        w.line("mix.ground", mix_ground_id(m.ground));
        for (name, on, r) in mix_pressure_items(m) {
            if on {
                w.bool(&format!("mix.{name}.pressure"), true);
            }
            if r.min() != 0.0 {
                w.single(&format!("mix.{name}.min"), r.min());
            }
            if !r.curve().is_empty() {
                w.line(&format!("mix.{name}.curve"), curve_text(r.curve()));
            }
        }
    }
    // 入り抜き・手ぶれ補正（版 4。持たないブラシは書かない）
    if let Some(a) = assist {
        w.line("assist.stabilizer", a.stabilizer);
        w.line("assist.taper_in", a.taper_in);
        w.line("assist.taper_out", a.taper_out);
    }
    match b.effect {
        BrushEffect::Paint => w.line("effect", "paint"),
        BrushEffect::Blur { radius } => {
            w.line("effect", "blur");
            w.line("effect.radius", radius);
        }
        BrushEffect::Smudge { strength } => {
            w.line("effect", "smudge");
            w.line("effect.strength", strength);
        }
        BrushEffect::Clone { offset } => {
            w.line("effect", "clone");
            w.line("effect.x", offset.x);
            w.line("effect.y", offset.y);
        }
    }
    if let Some(meta) = user.import.as_ref().map(ImportMeta::normalized) {
        w.line(
            "import.source",
            meta.source.replace(|c: char| c.is_control(), " "),
        );
        if meta.pattern {
            w.bool("import.pattern", true);
        }
        if !meta.gaps.is_empty() {
            let ids: Vec<&str> = meta.gaps.iter().map(|g| g.id()).collect();
            w.line("import.gaps", ids.join(","));
        }
        if !meta.mapped.is_empty() {
            let ids: Vec<&str> = meta
                .mapped
                .iter()
                .map(|m| super::gaps::mapped_id(*m))
                .collect();
            w.line("import.mapped", ids.join(","));
        }
    }
    Ok(Encoded {
        text: w.0,
        images: pending.images,
    })
}

/// 項目を取り出しながら読む（取り出さなかった項目は最後に「知らない項目」として断る）。
struct Reader {
    map: HashMap<String, String>,
}

impl Reader {
    fn take(&mut self, key: &str) -> Option<String> {
        self.map.remove(key)
    }
    fn parse<T: std::str::FromStr>(&mut self, key: &str, default: T) -> Result<T, StoreError> {
        match self.take(key) {
            None => Ok(default),
            Some(v) => v.parse().map_err(|_| StoreError::BadValue(key.into())),
        }
    }
    /// 有限の f64。
    fn float(&mut self, key: &str, default: f64) -> Result<f64, StoreError> {
        let v: f64 = self.parse(key, default)?;
        if v.is_finite() {
            Ok(v)
        } else {
            Err(StoreError::BadValue(key.into()))
        }
    }
    /// 画面の精度（f32）で書いた値。
    fn single(&mut self, key: &str, default: f64) -> Result<f64, StoreError> {
        let v: f32 = self.parse(key, default as f32)?;
        if v.is_finite() {
            Ok(v as f64)
        } else {
            Err(StoreError::BadValue(key.into()))
        }
    }
    fn bool(&mut self, key: &str, default: bool) -> Result<bool, StoreError> {
        match self.take(key).as_deref() {
            None => Ok(default),
            Some("1") => Ok(true),
            Some("0") => Ok(false),
            Some(_) => Err(StoreError::BadValue(key.into())),
        }
    }
    /// 画像 1 枚の札（1 枚に決まらない札は値が読めない）。
    fn tip(
        &mut self,
        key: &str,
        load: &mut dyn FnMut(&str) -> Result<Arc<BrushTip>, StoreError>,
    ) -> Result<Option<Arc<BrushTip>>, StoreError> {
        match self.take(key) {
            None => Ok(None),
            Some(token) if token == "none" => Ok(None),
            Some(token) => {
                let mut tips = resolve(&token, load)?;
                match (tips.pop(), tips.is_empty()) {
                    (Some(tip), true) => Ok(Some(tip)),
                    _ => Err(StoreError::BadValue(key.into())),
                }
            }
        }
    }
}

/// 筆圧の応えの項目（ファイルの名前と応え）。
fn pressure_items(b: &Brush) -> [(&'static str, &PressureResponse); 4] {
    [
        ("size", &b.pressure.size),
        ("opacity", &b.pressure.opacity),
        ("flow", &b.pressure.flow),
        ("hardness", &b.pressure.hardness),
    ]
}

/// 筆圧の応え（最小値・曲線・硬さの切り替え）を 1 つでも使うか（使えば版 2 で書く）。
fn uses_pressure_response(b: &Brush) -> bool {
    b.controls.pressure_hardness || !b.pressure.is_identity()
}

/// 色の混ぜを使うか（使えば版 3 で書く）。混ぜ方が切なら、ほかの値が既定でなくても使っていない（`canonical` が既定へそろえる）。
fn uses_mix(b: &Brush) -> bool {
    b.mix.is_active()
}

fn mix_mode_id(mode: MixMode) -> &'static str {
    match mode {
        MixMode::Off => "off",
        MixMode::Mix => "mix",
        MixMode::Smear => "smear",
    }
}

fn mix_ground_id(ground: MixGround) -> &'static str {
    match ground {
        MixGround::Layer => "layer",
        MixGround::Composite => "composite",
    }
}

/// 色の混ぜの筆圧の項目（ファイルの名前・筆圧で変えるか・応え）。
fn mix_pressure_items(m: &ColorMix) -> [(&'static str, bool, &PressureResponse); 2] {
    [
        ("paint", m.pressure_paint, &m.response_paint),
        ("density", m.pressure_density, &m.response_density),
    ]
}

/// 曲線の点を `x:y,x:y,…`（画面の精度 f32 の最短の表記）にする。
pub(crate) fn curve_text(points: &[CurvePoint]) -> String {
    let parts: Vec<String> = points
        .iter()
        .map(|p| format!("{}:{}", p.x as f32, p.y as f32))
        .collect();
    parts.join(",")
}

/// `x:y,x:y,…` を読む（f32 で書いた値。形が違えば None）。点の数は上限（16）の 1 つ上まで数え、多すぎる曲線は呼び手の検査が断る。
pub(crate) fn parse_curve(text: &str) -> Option<Vec<CurvePoint>> {
    let mut points = Vec::new();
    for part in text.split(',').take(MAX_CURVE_POINTS + 1) {
        let (x, y) = part.split_once(':')?;
        let (x, y): (f32, f32) = (x.parse().ok()?, y.parse().ok()?);
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        points.push(CurvePoint {
            x: x as f64,
            y: y as f64,
        });
    }
    Some(points)
}

fn texture_mode(id: &str) -> Option<TextureMode> {
    TextureMode::ALL.into_iter().find(|m| mode_id(*m) == id)
}

fn dual_mode(id: &str) -> Option<DualBrushMode> {
    DualBrushMode::ALL
        .into_iter()
        .find(|m| dual_mode_id(*m) == id)
}

/// 読んだブラシ 1 つ（ブラシは正規の形）。
#[derive(Clone, Debug, PartialEq)]
pub struct Decoded {
    pub name: String,
    pub group: Group,
    pub brush: Brush,
    pub import: Option<ImportMeta>,
    /// 入り抜き・手ぶれ補正を持つブラシの値（版 4）。
    pub assist: Option<StrokeAssist>,
}

/// ファイルの中身から、名前・グループ・ブラシ（正規の形）を読む。取り込んだ画像（`img:`）は読めない（`decode_user`）。
pub fn decode(text: &str) -> Result<(String, Group, Brush), StoreError> {
    let d = decode_user(text, &mut |hash| {
        Err(StoreError::UnknownTip(format!(
            "{IMAGE_PREFIX}{}",
            hash.chars().take(8).collect::<String>()
        )))
    })?;
    Ok((d.name, d.group, d.brush))
}

/// ファイルの中身から読む。取り込んだ画像は `load`（指紋 → 画像）で読む。
pub fn decode_user(
    text: &str,
    load: &mut dyn FnMut(&str) -> Result<Arc<BrushTip>, StoreError>,
) -> Result<Decoded, StoreError> {
    let mut lines = text.lines();
    let version = match lines.next() {
        Some(HEADER) => 1,
        Some(HEADER_V2) => 2,
        Some(HEADER_V3) => 3,
        Some(HEADER_V4) => 4,
        Some(HEADER_V5) => 5,
        Some(first) if first.starts_with("yolupainter-brush ") => {
            return Err(StoreError::NewerVersion(first.to_owned()))
        }
        _ => return Err(StoreError::NotABrush),
    };
    let mut map = HashMap::new();
    for (i, line) in lines.enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(StoreError::Syntax(i + 2))?;
        if map.insert(key.to_owned(), value.to_owned()).is_some() {
            return Err(StoreError::DuplicateKey(key.to_owned()));
        }
    }
    let mut r = Reader { map };
    let name = r.take("name").ok_or(StoreError::BadValue("name".into()))?;
    let name = super::clean_name(&name).ok_or(StoreError::BadValue("name".into()))?;
    let group = r
        .take("group")
        .and_then(|g| Group::from_id(&g))
        .ok_or(StoreError::BadValue("group".into()))?;
    let mut b = Brush::default();
    b.base.radius = r.single("radius", b.base.radius)?;
    b.base.hardness = r.single("hardness", b.base.hardness)?;
    b.base.spacing = r.single("spacing", b.base.spacing)?;
    b.base.opacity = r.single("opacity", b.base.opacity)?;
    b.base.flow = r.single("flow", b.base.flow)?;
    b.base.pressure_size = r.bool("pressure_size", b.base.pressure_size)?;
    b.base.pressure_opacity = r.bool("pressure_opacity", b.base.pressure_opacity)?;
    b.base.pressure_flow = r.bool("pressure_flow", b.base.pressure_flow)?;
    // 縁のアンチエイリアスは版 5 の項目（版 4 までのファイルにあれば、知らない項目として断る。無ければ なし）
    if version >= 5 {
        if let Some(id) = r.take("anti_alias") {
            b.base.anti_alias =
                AntiAlias::from_id(&id).ok_or(StoreError::BadValue("anti_alias".into()))?;
        }
    }
    b.tip.image = r.tip("tip.image", load)?;
    if let Some(list) = r.take("tip.images") {
        if list != "none" {
            for token in list.split(',') {
                b.tip.images.extend(resolve(token, load)?);
            }
        }
    }
    b.tip.selection = match r.take("tip.selection").as_deref() {
        None | Some("random") => TipSelection::Random,
        Some("sequential") => TipSelection::Sequential,
        Some(_) => return Err(StoreError::BadValue("tip.selection".into())),
    };
    b.tip.angle = r.float("tip.angle", b.tip.angle)?;
    b.tip.roundness = r.float("tip.roundness", b.tip.roundness)?;
    b.tip.follow_direction = r.bool("tip.follow", b.tip.follow_direction)?;
    b.tip.flip_x = r.bool("tip.flip_x", b.tip.flip_x)?;
    b.tip.flip_y = r.bool("tip.flip_y", b.tip.flip_y)?;
    b.jitter.size = r.float("jitter.size", b.jitter.size)?;
    b.jitter.angle = r.float("jitter.angle", b.jitter.angle)?;
    b.jitter.roundness = r.float("jitter.roundness", b.jitter.roundness)?;
    b.jitter.opacity = r.float("jitter.opacity", b.jitter.opacity)?;
    b.jitter.flow = r.float("jitter.flow", b.jitter.flow)?;
    b.jitter.scatter = r.float("jitter.scatter", b.jitter.scatter)?;
    b.jitter.count = r.parse("jitter.count", b.jitter.count)?;
    if let Some(image) = r.tip("texture", load)? {
        let mut texture = PaperTexture::new(image, 0.5);
        texture.depth = r.float("texture.depth", texture.depth)?;
        texture.scale = r.float("texture.scale", texture.scale)?;
        if let Some(mode) = r.take("texture.mode") {
            texture.mode =
                texture_mode(&mode).ok_or(StoreError::BadValue("texture.mode".into()))?;
        }
        b.texture = Some(texture);
    }
    if r.bool("dual", false)? {
        let mut dual = DualBrush::default();
        dual.tip = r.tip("dual.tip", load)?;
        dual.radius = r.float("dual.radius", dual.radius)?;
        dual.hardness = r.float("dual.hardness", dual.hardness)?;
        dual.spacing = r.float("dual.spacing", dual.spacing)?;
        dual.angle = r.float("dual.angle", dual.angle)?;
        dual.roundness = r.float("dual.roundness", dual.roundness)?;
        dual.scatter = r.float("dual.scatter", dual.scatter)?;
        dual.count = r.parse("dual.count", dual.count)?;
        if let Some(mode) = r.take("dual.mode") {
            dual.mode = dual_mode(&mode).ok_or(StoreError::BadValue("dual.mode".into()))?;
        }
        b.dual = Some(dual);
    }
    b.color.foreground_background = r.float("color.fg_bg", b.color.foreground_background)?;
    b.color.hue = r.float("color.hue", b.color.hue)?;
    b.color.saturation = r.float("color.saturation", b.color.saturation)?;
    b.color.brightness = r.float("color.brightness", b.color.brightness)?;
    b.color.purity = r.float("color.purity", b.color.purity)?;
    b.color.per_tip = r.bool("color.per_tip", b.color.per_tip)?;
    let k = &mut b.controls;
    k.fade_size = r.parse("controls.fade_size", k.fade_size)?;
    k.fade_opacity = r.parse("controls.fade_opacity", k.fade_opacity)?;
    k.fade_flow = r.parse("controls.fade_flow", k.fade_flow)?;
    k.tilt_size = r.bool("controls.tilt_size", k.tilt_size)?;
    k.tilt_opacity = r.bool("controls.tilt_opacity", k.tilt_opacity)?;
    k.tilt_flow = r.bool("controls.tilt_flow", k.tilt_flow)?;
    k.tilt_angle = r.bool("controls.tilt_angle", k.tilt_angle)?;
    k.rotation_angle = r.bool("controls.rotation_angle", k.rotation_angle)?;
    k.speed_size = r.bool("controls.speed_size", k.speed_size)?;
    k.speed_opacity = r.bool("controls.speed_opacity", k.speed_opacity)?;
    k.speed_flow = r.bool("controls.speed_flow", k.speed_flow)?;
    k.speed_max = r.float("controls.speed_max", k.speed_max)?;
    // 筆圧の応えは版 2 の項目（版 1 のファイルにあれば、知らない項目として断る）
    if version >= 2 {
        b.controls.pressure_hardness = r.bool("controls.pressure_hardness", false)?;
        for (name, _) in pressure_items(&Brush::default()) {
            let min = r.single(&format!("pressure.{name}.min"), 0.0)?;
            let curve_key = format!("pressure.{name}.curve");
            let curve = match r.take(&curve_key) {
                Some(text) => parse_curve(&text).ok_or(StoreError::BadValue(curve_key))?,
                None => Vec::new(),
            };
            let response = PressureResponse::new(min, curve).map_err(StoreError::Invalid)?;
            match name {
                "size" => b.pressure.size = response,
                "opacity" => b.pressure.opacity = response,
                "flow" => b.pressure.flow = response,
                _ => b.pressure.hardness = response,
            }
        }
    }
    // 色の混ぜは版 3 の項目（版 1・2 のファイルにあれば、知らない項目として断る）
    if version >= 3 {
        b.mix = read_mix(&mut r)?;
    }
    // 入り抜き・手ぶれ補正は版 4 の項目（版 3 までのファイルにあれば、知らない項目として断る）。版 4 でも、1 つも無ければ持たない
    let assist = if version >= 4 {
        let keys = ["assist.stabilizer", "assist.taper_in", "assist.taper_out"];
        let present = keys.iter().any(|k| r.map.contains_key(*k));
        let value = |r: &mut Reader, key: &str| r.float(key, 0.0);
        let (stabilizer, taper_in, taper_out) = (
            value(&mut r, keys[0])?,
            value(&mut r, keys[1])?,
            value(&mut r, keys[2])?,
        );
        present.then_some(StrokeAssist {
            stabilizer,
            taper_in,
            taper_out,
            curve: false,
        })
    } else {
        None
    };
    b.effect = match r.take("effect").as_deref() {
        None | Some("paint") => BrushEffect::Paint,
        Some("blur") => BrushEffect::Blur {
            radius: r.parse("effect.radius", 3)?,
        },
        Some("smudge") => BrushEffect::Smudge {
            strength: r.float("effect.strength", 0.5)?,
        },
        Some("clone") => BrushEffect::Clone {
            offset: DVec2::new(r.float("effect.x", 0.0)?, r.float("effect.y", 0.0)?),
        },
        Some(_) => return Err(StoreError::BadValue("effect".into())),
    };
    let import = {
        let source = r.take("import.source");
        let pattern = r.take("import.pattern");
        let gaps = r.take("import.gaps");
        let mapped = r.take("import.mapped");
        if source.is_none() && pattern.is_none() && gaps.is_none() && mapped.is_none() {
            None
        } else {
            let source: String = source
                .unwrap_or_default()
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>()
                .trim()
                .chars()
                .take(MAX_SOURCE_CHARS)
                .collect();
            let pattern = match pattern.as_deref() {
                None | Some("0") => false,
                Some("1") => true,
                Some(_) => return Err(StoreError::BadValue("import.pattern".into())),
            };
            // 知らない項目の名前は読み飛ばす（新しい版が足した項目。描き方には関わらない情報）
            let gaps = gaps
                .unwrap_or_default()
                .split(',')
                .filter_map(Gap::from_id)
                .collect();
            let mapped = mapped
                .unwrap_or_default()
                .split(',')
                .filter_map(super::gaps::mapped_from_id)
                .collect();
            Some(ImportMeta::new(source, pattern, gaps).with_mapped(mapped))
        }
    };
    if let Some(key) = r.map.keys().min() {
        return Err(StoreError::UnknownKey(key.clone()));
    }
    // 範囲（有限・0〜10000）は core の検査に任せる
    if let Some(a) = assist {
        b.assist = a;
    }
    b.validate().map_err(StoreError::Invalid)?;
    Ok(Decoded {
        name,
        group,
        brush: canonical(&b),
        import,
        assist: carried_assist(assist),
    })
}

/// 版 3 の色の混ぜの項目を読む（無い項目は既定）。
fn read_mix(r: &mut Reader) -> Result<ColorMix, StoreError> {
    let mut m = ColorMix::default();
    m.mode = match r.take("mix.mode").as_deref() {
        None | Some("off") => MixMode::Off,
        Some("mix") => MixMode::Mix,
        Some("smear") => MixMode::Smear,
        Some(_) => return Err(StoreError::BadValue("mix.mode".into())),
    };
    m.paint = r.single("mix.paint", m.paint)?;
    m.density = r.single("mix.density", m.density)?;
    m.stretch = r.single("mix.stretch", m.stretch)?;
    m.ground = match r.take("mix.ground").as_deref() {
        None | Some("layer") => MixGround::Layer,
        Some("composite") => MixGround::Composite,
        Some(_) => return Err(StoreError::BadValue("mix.ground".into())),
    };
    for name in ["paint", "density"] {
        let on = r.bool(&format!("mix.{name}.pressure"), false)?;
        let min = r.single(&format!("mix.{name}.min"), 0.0)?;
        let curve_key = format!("mix.{name}.curve");
        let curve = match r.take(&curve_key) {
            Some(text) => parse_curve(&text).ok_or(StoreError::BadValue(curve_key))?,
            None => Vec::new(),
        };
        let response = PressureResponse::new(min, curve).map_err(StoreError::Invalid)?;
        if name == "paint" {
            m.pressure_paint = on;
            m.response_paint = response;
        } else {
            m.pressure_density = on;
            m.response_density = response;
        }
    }
    Ok(m)
}

// ───────── フォルダ ─────────

/// `brush-<8 桁の 16 進>.ylbrush` の番号。
fn id_from_file(name: &str) -> Option<u32> {
    let hex = name
        .strip_prefix("brush-")?
        .strip_suffix(&format!(".{EXTENSION}"))?;
    if hex.len() != 8 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

fn file_name(id: u32) -> String {
    format!("brush-{id:08x}.{EXTENSION}")
}

/// 大きさを上限で止めて読む。
fn read_text(path: &Path) -> Result<String, StoreError> {
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(text)
}

/// 大きさを上限で止めて、バイト列で読む。
fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>, StoreError> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(StoreError::TooLarge);
    }
    Ok(bytes)
}

/// 一時ファイルに書いて同期し、`verify` が読み戻しを確かめたら `path` へ置き換える。失敗したら一時ファイルを消す。
fn replace_file(path: &Path, text: &str, verify: impl Fn(&str) -> bool) -> Result<(), StoreError> {
    replace_bytes(path, text.as_bytes(), MAX_FILE_BYTES, |read| {
        std::str::from_utf8(read).is_ok_and(&verify)
    })
}

/// 設定のフォルダの文のファイルを置く（一時ファイルへ書いて読み戻して確かめ、最後の 1 回の置換で確定）。ブラシ以外の設定のファイル
/// （サブツールのプリセット）も、同じ置き方を使う。`limit` はそのファイルの大きさの上限（読み戻しもここまで。超えれば `TooLarge`）。
pub(crate) fn replace_text(
    path: &Path,
    text: &str,
    limit: u64,
    verify: impl Fn(&str) -> bool,
) -> Result<(), StoreError> {
    replace_bytes(path, text.as_bytes(), limit, |read| {
        std::str::from_utf8(read).is_ok_and(&verify)
    })
}

/// `replace_file` のバイト列版（画像）。読み戻しは `limit` バイトまで（超えれば `TooLarge`）。
fn replace_bytes(
    path: &Path,
    bytes: &[u8],
    limit: u64,
    verify: impl Fn(&[u8]) -> bool,
) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "brush directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let opts = yolu_io::atomic::ReplaceOptions {
        limit: Some(limit),
        verify: Some(&verify),
        create_dirs: false,
    };
    yolu_io::atomic::replace_with(path, &opts, |f| f.write_all(bytes)).map_err(|e| {
        match yolu_io::atomic::rejected(&e) {
            Some(yolu_io::atomic::Rejected::TooLarge) => StoreError::TooLarge,
            Some(yolu_io::atomic::Rejected::Mismatch) => StoreError::Mismatch,
            None => StoreError::Io(e),
        }
    })
}

/// ブラシのファイルの文が指す取り込んだ画像の指紋（文字として探す。読めない値のファイルでも数える）。
fn image_hashes(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for line in text.lines() {
        let Some((_, value)) = line.split_once('=') else {
            continue;
        };
        for token in value.split(',') {
            if let Some(hash) = token.trim().strip_prefix(IMAGE_PREFIX) {
                if images::is_fingerprint(hash) && !found.iter().any(|h| h == hash) {
                    found.push(hash.to_owned());
                }
            }
        }
    }
    found
}

/// 設定のフォルダの `brushes/`。
#[derive(Clone, Debug)]
pub struct BrushStore {
    dir: PathBuf,
    /// この実行で、置いた（読み戻して確かめた）画像の指紋。同じ筆先を使うブラシをまとめて置くとき・名前を変えて書き直すときに、
    /// 画像を読み直さない（複製したストアも共有する。別のスレッドの取り込みの仕事と画面の側で同じ集合を使う）。
    placed: Arc<std::sync::Mutex<HashSet<String>>>,
}

impl BrushStore {
    pub fn new(dir: PathBuf) -> BrushStore {
        BrushStore {
            dir,
            placed: Arc::default(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_of(&self, id: u32) -> PathBuf {
        self.dir.join(file_name(id))
    }

    /// この番号のファイル（読めない・ファイルでないものも）がもうあるか。新しいブラシの保存は置換なので、あるなら使わない。
    pub fn is_taken(&self, id: u32) -> bool {
        std::fs::symlink_metadata(self.path_of(id)).is_ok()
    }

    /// 取り込んだ画像のフォルダ。
    pub fn images_dir(&self) -> PathBuf {
        self.dir.join("images")
    }

    pub fn image_path(&self, hash: &str) -> PathBuf {
        self.images_dir().join(format!("{hash}.png"))
    }

    /// 取り込んだ画像を読む（指紋と中身が合わなければ断る）。ファイルが無ければ知らない画像。
    pub fn load_image(&self, hash: &str) -> Result<BrushTip, StoreError> {
        let short = || hash.chars().take(8).collect::<String>();
        if !images::is_fingerprint(hash) {
            return Err(StoreError::BadImage(short()));
        }
        let bytes = match read_bytes(&self.image_path(hash), images::MAX_IMAGE_FILE_BYTES) {
            Ok(bytes) => bytes,
            Err(StoreError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::UnknownTip(format!("{IMAGE_PREFIX}{}", short())))
            }
            Err(StoreError::TooLarge) => return Err(StoreError::BadImage(short())),
            Err(e) => return Err(e),
        };
        let tip = images::decode(&bytes).map_err(|_| StoreError::BadImage(short()))?;
        if images::fingerprint(&tip) != hash {
            return Err(StoreError::BadImage(short()));
        }
        Ok(tip)
    }

    /// 画像を置く（同じ指紋のファイルが読めるなら、そのまま）。一時ファイルへ書いて読み戻して確かめてから置換する。
    /// この実行で置いたものは、ファイルがまだあれば確かめ直さない。
    fn save_image(&self, hash: &str, tip: &BrushTip) -> Result<(), StoreError> {
        let placed = |s: &BrushStore| s.placed.lock().is_ok_and(|set| set.contains(hash));
        if (placed(self) && self.image_path(hash).is_file()) || self.load_image(hash).is_ok() {
            if let Ok(mut set) = self.placed.lock() {
                set.insert(hash.to_owned());
            }
            return Ok(());
        }
        let png = images::encode(tip)?;
        replace_bytes(
            &self.image_path(hash),
            &png,
            images::MAX_IMAGE_FILE_BYTES,
            |read| images::decode(read).is_ok_and(|got| got == *tip),
        )?;
        if let Ok(mut set) = self.placed.lock() {
            set.insert(hash.to_owned());
        }
        Ok(())
    }

    /// ブラシを置く。画像を先に、ブラシのファイルを最後の 1 回の置換で。
    pub fn save_brush(&self, user: &UserBrush) -> Result<(), StoreError> {
        let encoded = encode(user)?;
        for (hash, tip) in &encoded.images {
            self.save_image(hash, tip)?;
        }
        let expect = Decoded {
            name: super::clean_name(&user.name).unwrap_or_default(),
            group: user.group,
            brush: canonical(&user.brush),
            import: user.import.as_ref().map(ImportMeta::normalized),
            assist: carried_assist(user.assist),
        };
        // 読み戻しの確かめでは、置いたばかりの画像は手元のものを使う（画像は置くときに読み戻して確かめ済み）
        let have: HashMap<&str, &Arc<BrushTip>> = encoded
            .images
            .iter()
            .map(|(hash, tip)| (hash.as_str(), tip))
            .collect();
        replace_file(&self.path_of(user.id), &encoded.text, |read| {
            let mut load = |hash: &str| {
                have.get(hash).map(|tip| Arc::clone(tip)).ok_or_else(|| {
                    StoreError::UnknownTip(format!(
                        "{IMAGE_PREFIX}{}",
                        hash.chars().take(8).collect::<String>()
                    ))
                })
            };
            decode_user(read, &mut load).is_ok_and(|got| got == expect)
        })
    }

    /// ブラシのファイルを消す。そのブラシだけが使っていた取り込んだ画像も消す（ほかのブラシのファイルが 1 つでも指していれば残す。
    /// 読めないファイル・新しい版のファイルも、文字として探して数える）。画像を消せなくてもブラシを消したことは変わらない。
    pub fn delete_brush(&self, id: u32) -> Result<(), StoreError> {
        let path = self.path_of(id);
        let used = read_text(&path)
            .map(|text| image_hashes(&text))
            .unwrap_or_default();
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if !used.is_empty() {
            self.remove_unused_images(used);
        }
        Ok(())
    }

    fn remove_unused_images(&self, mut candidates: Vec<String>) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if id_from_file(&name).is_none() {
                continue;
            }
            // 読めないファイルがあれば、何が指しているか分からないので、何も消さない
            let Ok(text) = read_text(&entry.path()) else {
                return;
            };
            let kept = image_hashes(&text);
            candidates.retain(|hash| !kept.contains(hash));
            if candidates.is_empty() {
                return;
            }
        }
        for hash in candidates {
            if std::fs::remove_file(self.image_path(&hash)).is_ok() {
                if let Ok(mut set) = self.placed.lock() {
                    set.remove(&hash);
                }
            }
        }
    }

    pub fn save_order(&self, tokens: &[String]) -> Result<(), StoreError> {
        let text: String = tokens.iter().map(|t| format!("{t}\n")).collect();
        replace_file(&self.dir.join(ORDER_FILE), &text, |read| read == text)
    }
}

/// フォルダのブラシを全部読む。読めないファイルは読み飛ばして `problems` に残す。フォルダが無ければ空。
pub fn load_all(dir: &Path) -> LoadReport {
    let mut report = LoadReport::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return report,
        Err(e) => {
            report.problems.push(Problem {
                file: dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                reason: e.into(),
            });
            return report;
        }
    };
    let mut files: Vec<(u32, String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_file = e.file_type().is_ok_and(|t| t.is_file());
            id_from_file(&name).map(|id| (id, name, is_file))
        })
        .collect();
    files.sort();
    report.max_file_id = files.iter().map(|(id, _, _)| *id).max();
    files.retain(|(_, _, is_file)| *is_file);
    let on_disk: Vec<u32> = files.iter().map(|(id, _, _)| *id).collect();
    if files.len() > MAX_FILES {
        report.problems.push(Problem {
            file: dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            reason: StoreError::TooMany,
        });
        files.truncate(MAX_FILES);
    }
    // 取り込んだ画像は指紋ごとに 1 回だけ読み、同じ画像を使うブラシで共有する
    let store = BrushStore::new(dir.to_path_buf());
    let mut shared: HashMap<String, Arc<BrushTip>> = HashMap::new();
    let mut load = |hash: &str| -> Result<Arc<BrushTip>, StoreError> {
        if let Some(tip) = shared.get(hash) {
            return Ok(tip.clone());
        }
        let tip = Arc::new(store.load_image(hash)?);
        shared.insert(hash.to_owned(), tip.clone());
        Ok(tip)
    };
    for (id, name, _) in files {
        let loaded = read_text(&dir.join(&name)).and_then(|text| decode_user(&text, &mut load));
        match loaded {
            Ok(d) => report.brushes.push(UserBrush {
                id,
                name: d.name,
                group: d.group,
                brush: d.brush,
                import: d.import,
                assist: d.assist,
            }),
            Err(reason) => report.problems.push(Problem { file: name, reason }),
        }
    }
    let loaded: HashSet<u32> = report.brushes.iter().map(|b| b.id).collect();
    report.unloaded = on_disk
        .into_iter()
        .filter(|id| !loaded.contains(id))
        .collect();
    match read_text(&dir.join(ORDER_FILE)) {
        Ok(text) => {
            report.order = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .take(MAX_FILES + 64)
                .map(str::to_owned)
                .collect()
        }
        Err(StoreError::Io(e)) if e.kind() == io::ErrorKind::NotFound => {}
        Err(reason) => report.problems.push(Problem {
            file: ORDER_FILE.into(),
            reason,
        }),
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Jitter, TipShape};
    use crate::m2;
    use yolu_core::brush::TipSelection;

    fn user(id: u32, brush: Brush) -> UserBrush {
        UserBrush {
            id,
            name: format!("テスト {id}"),
            group: Group::Pen,
            brush: canonical(&brush),
            import: None,
            assist: None,
        }
    }

    /// ブラシの文だけ（画像のファイルを要しないブラシ）。
    fn text(user: &UserBrush) -> String {
        let encoded = encode(user).unwrap();
        assert!(encoded.images.is_empty(), "画像のファイルを要しない");
        encoded.text
    }

    #[test]
    fn every_builtin_round_trips_through_the_text_format() {
        for b in super::super::builtin::all() {
            let u = UserBrush {
                id: 1,
                name: "x".into(),
                group: b.group,
                brush: b.brush.clone(),
                import: None,
                assist: None,
            };
            let text = encode(&u)
                .unwrap_or_else(|e| panic!("{}: {e:?}", b.id))
                .text;
            let (name, group, brush) =
                decode(&text).unwrap_or_else(|e| panic!("{}: {e:?}\n{text}", b.id));
            assert_eq!((name.as_str(), group), ("x", b.group), "{}", b.id);
            assert_eq!(brush, b.brush, "{}", b.id);
        }
        for p in m2::presets() {
            let u = user(2, p.brush.clone());
            assert_eq!(decode(&text(&u)).unwrap().2, u.brush, "{}", p.id);
        }
    }

    #[test]
    fn a_full_featured_brush_round_trips_and_text_is_stable() {
        let mut b = Brush::default();
        b.base.radius = 33.5;
        b.base.hardness = 0.37;
        b.base.pressure_flow = true;
        b.tip = TipShape {
            image: builtin_tip("bristles"),
            angle: -42.5,
            roundness: 0.4,
            follow_direction: true,
            flip_x: true,
            ..TipShape::default()
        };
        b.jitter = Jitter {
            size: 0.25,
            scatter: 2.5,
            count: 4,
            ..Jitter::default()
        };
        b.texture = Some(PaperTexture {
            scale: 3.25,
            mode: TextureMode::Overlay,
            ..PaperTexture::new(builtin_tip("grain").unwrap(), 0.7)
        });
        b.dual = Some(DualBrush {
            tip: builtin_tip("dots"),
            radius: 5.0,
            mode: DualBrushMode::Darken,
            count: 3,
            ..DualBrush::default()
        });
        b.color.hue = 0.2;
        b.color.purity = -0.5;
        b.color.per_tip = false;
        b.controls.fade_size = 120;
        b.controls.tilt_angle = true;
        b.controls.speed_max = 1500.0;
        b.effect = BrushEffect::Clone {
            offset: DVec2::new(-12.5, 40.0),
        };
        let u = user(9, b);
        let text = text(&u);
        let (_, _, back) = decode(&text).unwrap();
        assert_eq!(back, u.brush);
        // 書き直しても同じ文
        let again = self::text(&UserBrush {
            brush: back,
            ..u.clone()
        });
        assert_eq!(again, text);
        assert!(text.starts_with("yolupainter-brush 1\n"), "{text}");
    }

    use yolu_core::generator::CurvePoint;

    fn pt(x: f64, y: f64) -> CurvePoint {
        CurvePoint { x, y }
    }

    /// 筆圧の応えを全項目に入れたブラシ。
    fn pressure_brush() -> Brush {
        let mut b = Brush::default();
        b.base.pressure_flow = true;
        b.controls.pressure_hardness = true;
        b.pressure.size = PressureResponse::new(0.25, vec![]).unwrap();
        b.pressure.opacity = PressureResponse::new(
            0.1,
            vec![pt(0.0, 0.0), pt(0.4, 0.7), pt(0.75, 0.8), pt(1.0, 1.0)],
        )
        .unwrap();
        b.pressure.flow = PressureResponse::new(0.0, vec![pt(0.0, 1.0), pt(1.0, 0.2)]).unwrap();
        b.pressure.hardness = PressureResponse::new(0.333, vec![]).unwrap();
        b
    }

    #[test]
    fn a_brush_with_a_pressure_response_is_version_two_and_round_trips() {
        let u = user(5, pressure_brush());
        let t = text(&u);
        assert!(t.starts_with("yolupainter-brush 2\n"), "{t}");
        for key in [
            "controls.pressure_hardness=1",
            "pressure.size.min=0.25",
            "pressure.opacity.min=0.1",
            "pressure.opacity.curve=0:0,0.4:0.7,0.75:0.8,1:1",
            "pressure.flow.curve=0:1,1:0.2",
            "pressure.hardness.min=0.333",
        ] {
            assert!(t.lines().any(|l| l == key), "{key}\n{t}");
        }
        // 既定の項目（最小値 0・直線）は書かない
        assert!(!t.contains("pressure.size.curve"), "{t}");
        assert!(!t.contains("pressure.flow.min"), "{t}");
        let (_, _, back) = decode(&t).unwrap();
        assert_eq!(back, u.brush);
        let again = text(&UserBrush { brush: back, ..u });
        assert_eq!(again, t);
    }

    #[test]
    fn a_curve_made_with_the_editor_keeps_its_shape_through_the_file() {
        // 編集の部品が作る曲線は f64 の半端な値を持つ。ファイルは f32 で書くので、読み戻した値は画面の精度に丸めたものと同じで、何度書いても変わらない
        use crate::ui::curve::ops;
        use yolu_core::curve::Curve;
        let (c, k) = ops::add_point(&Curve::identity(), 0.373_737_373_7, 0.616_161_616_1).unwrap();
        let c = ops::move_point(&c, k, 0.412_345_678_91, 0.777_777_777_7).unwrap();
        let mut b = Brush::default();
        b.pressure.size = PressureResponse::new(0.123_456_789, vec![])
            .unwrap()
            .with_curve_shape(c.clone())
            .unwrap();
        let u = user(2, b);
        let t = text(&u);
        assert!(t.starts_with("yolupainter-brush 2\n"), "{t}");
        let (_, _, back) = decode(&t).unwrap();
        assert_eq!(back, canonical(&u.brush));
        assert_eq!(back.pressure.size.curve().len(), 3);
        for (saved, made) in back.pressure.size.curve().iter().zip(c.points()) {
            assert!(
                (saved.x - made.x).abs() < 1e-6 && (saved.y - made.y).abs() < 1e-6,
                "{saved:?} {made:?}"
            );
        }
        let again = text(&UserBrush {
            brush: back.clone(),
            ..u
        });
        assert_eq!(again, t);
        assert_eq!(decode(&again).unwrap().2, back);
    }

    #[test]
    fn a_brush_without_a_pressure_response_keeps_version_one_and_its_bytes() {
        // 筆圧の切り替えだけ（応えは既定）なら今までと同じ版と同じ行
        let mut b = Brush::default();
        b.base.pressure_flow = true;
        let t = text(&user(1, b));
        assert!(t.starts_with("yolupainter-brush 1\n"), "{t}");
        assert!(!t.contains("pressure."), "{t}");
        assert!(!t.contains("pressure_hardness"), "{t}");
        // 直線の曲線を明示しても既定と同じ
        let mut straight = Brush::default();
        straight.pressure.size =
            PressureResponse::new(0.0, vec![pt(0.0, 0.0), pt(1.0, 1.0)]).unwrap();
        assert_eq!(text(&user(1, straight)), text(&user(1, Brush::default())));
    }

    #[test]
    fn pressure_items_belong_to_version_two_and_are_checked() {
        let v2 = text(&user(1, pressure_brush()));
        // 版 1 のファイルにあれば知らない項目
        let as_v1 = v2.replacen("yolupainter-brush 2", "yolupainter-brush 1", 1);
        assert!(matches!(
            decode(&as_v1).unwrap_err(),
            StoreError::UnknownKey(k) if k.starts_with("pressure.") || k.starts_with("controls.pressure")
        ));
        // 版 2 でも、項目が無ければ既定
        let plain = text(&user(1, Brush::default())).replacen(
            "yolupainter-brush 1",
            "yolupainter-brush 2",
            1,
        );
        assert_eq!(decode(&plain).unwrap().2, canonical(&Brush::default()));
        let bad = |from: &str, to: &str| decode(&v2.replace(from, to)).unwrap_err();
        assert!(matches!(
            bad("pressure.size.min=0.25", "pressure.size.min=lots"),
            StoreError::BadValue(k) if k == "pressure.size.min"
        ));
        assert!(matches!(
            bad("pressure.size.min=0.25", "pressure.size.min=1.5"),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            bad("pressure.flow.curve=0:1,1:0.2", "pressure.flow.curve=0:1;1:0.2"),
            StoreError::BadValue(k) if k == "pressure.flow.curve"
        ));
        assert!(matches!(
            bad(
                "pressure.flow.curve=0:1,1:0.2",
                "pressure.flow.curve=0:1,0.5:2,1:0.2"
            ),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            bad("pressure.flow.curve=0:1,1:0.2", "pressure.flow.curve=0:1,0.5:NaN,1:0.2"),
            StoreError::BadValue(k) if k == "pressure.flow.curve"
        ));
        assert!(matches!(
            bad(
                "pressure.flow.curve=0:1,1:0.2",
                "pressure.flow.curve=0:1,0.5:0.5"
            ),
            StoreError::Invalid(_)
        ));
        let many: Vec<String> = (0..40)
            .map(|i| format!("{}:0.5", i as f64 / 39.0))
            .collect();
        assert!(matches!(
            bad(
                "pressure.flow.curve=0:1,1:0.2",
                &format!("pressure.flow.curve={}", many.join(","))
            ),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            decode(&format!("{v2}pressure.size.min=0.5\n")).unwrap_err(),
            StoreError::DuplicateKey(_)
        ));
        assert!(matches!(
            decode(&v2.replace(
                "controls.pressure_hardness=1",
                "controls.pressure_hardness=maybe"
            ))
            .unwrap_err(),
            StoreError::BadValue(_)
        ));
    }

    /// 色の混ぜを全項目に入れたブラシ（筆圧の応えも使う）。
    fn mixing_brush() -> Brush {
        Brush {
            mix: ColorMix {
                mode: MixMode::Smear,
                paint: 0.625,
                density: 0.8,
                stretch: 0.25,
                ground: MixGround::Composite,
                pressure_paint: true,
                pressure_density: true,
                response_paint: PressureResponse::new(0.2, vec![]).unwrap(),
                response_density: PressureResponse::new(
                    0.0,
                    vec![pt(0.0, 0.0), pt(0.5, 0.75), pt(1.0, 1.0)],
                )
                .unwrap(),
            },
            ..Brush::default()
        }
    }

    #[test]
    fn a_brush_that_mixes_is_version_three_and_round_trips() {
        let u = user(5, mixing_brush());
        let t = text(&u);
        assert!(t.starts_with("yolupainter-brush 3\n"), "{t}");
        for key in [
            "mix.mode=smear",
            "mix.paint=0.625",
            "mix.density=0.8",
            "mix.stretch=0.25",
            "mix.ground=composite",
            "mix.paint.pressure=1",
            "mix.paint.min=0.2",
            "mix.density.pressure=1",
            "mix.density.curve=0:0,0.5:0.75,1:1",
        ] {
            assert!(t.lines().any(|l| l == key), "{key}\n{t}");
        }
        // 既定の項目（最小値 0・直線）は書かない
        assert!(!t.contains("mix.paint.curve"), "{t}");
        assert!(!t.contains("mix.density.min"), "{t}");
        let (_, _, back) = decode(&t).unwrap();
        assert_eq!(back, u.brush);
        assert_eq!(text(&UserBrush { brush: back, ..u }), t);
        // 混ぜる + 筆圧の応えは、同じ版 3 のファイルに両方の項目を書く
        let mut both = mixing_brush();
        both.controls.pressure_hardness = true;
        both.pressure.size = PressureResponse::new(0.25, vec![]).unwrap();
        let t = text(&user(6, both.clone()));
        assert!(t.starts_with("yolupainter-brush 3\n"), "{t}");
        assert!(t.lines().any(|l| l == "pressure.size.min=0.25"), "{t}");
        assert_eq!(decode(&t).unwrap().2, canonical(&both));
        // 値は画面の精度（f32）で書く: 半端な値は丸めた値と同じになり、何度書いても変わらない
        let mut odd = mixing_brush();
        odd.mix.paint = 0.123_456_79;
        let u = user(7, odd);
        let t = text(&u);
        let back = decode(&t).unwrap().2;
        assert_eq!(back, u.brush);
        assert_eq!(back.mix.paint, 0.123_456_79_f32 as f64);
    }

    #[test]
    fn a_brush_that_does_not_mix_keeps_its_version_and_its_bytes() {
        let plain = text(&user(1, Brush::default()));
        assert!(plain.starts_with("yolupainter-brush 1\n"), "{plain}");
        assert!(!plain.contains("mix."), "{plain}");
        // 混ぜ方が切なら、ほかの値が既定でなくても使っていない: 版もバイトも混ぜを足す前と同じ
        let mut off = Brush::default();
        off.mix.paint = 0.1;
        off.mix.density = 0.2;
        off.mix.stretch = 0.9;
        off.mix.ground = MixGround::Composite;
        off.mix.pressure_paint = true;
        assert_eq!(text(&user(1, off)), plain);
        // 筆圧の応えだけのブラシは版 2 のまま、混ぜの行は無い
        let t = text(&user(1, pressure_brush()));
        assert!(t.starts_with("yolupainter-brush 2\n"), "{t}");
        assert!(!t.contains("mix."), "{t}");
        // 組み込みの全部が、混ぜを使わない（版 3 で書かれるのは厚塗りのブラシだけ）
        for b in super::super::builtin::all() {
            let u = UserBrush {
                id: 1,
                name: "x".into(),
                group: b.group,
                brush: b.brush.clone(),
                import: None,
                assist: None,
            };
            let t = encode(&u).unwrap().text;
            assert_eq!(
                t.contains("\nmix.mode="),
                b.brush.mix.is_active(),
                "{}",
                b.id
            );
            // 版 3 になるのは、縁のアンチエイリアスの無い（版 5 にならない）厚塗りのブラシだけ
            assert_eq!(
                t.starts_with("yolupainter-brush 3\n"),
                b.brush.mix.is_active()
                    && b.brush.base.anti_alias == crate::engine::AntiAlias::None,
                "{}",
                b.id
            );
        }
    }

    #[test]
    fn mix_items_belong_to_version_three_and_are_checked() {
        let v3 = text(&user(1, mixing_brush()));
        // 版 1・2 のファイルにあれば知らない項目（今までの版の読み手と同じ断り方）
        for old in ["yolupainter-brush 1", "yolupainter-brush 2"] {
            let as_old = v3.replacen("yolupainter-brush 3", old, 1);
            assert!(
                matches!(
                    decode(&as_old).unwrap_err(),
                    StoreError::UnknownKey(k) if k.starts_with("mix.") || k.starts_with("pressure.")
                ),
                "{old}"
            );
        }
        // 版 3 でも、項目が無ければ既定（混ぜない）
        let plain = text(&user(1, Brush::default())).replacen(
            "yolupainter-brush 1",
            "yolupainter-brush 3",
            1,
        );
        assert_eq!(decode(&plain).unwrap().2, canonical(&Brush::default()));
        // 今までの版のファイルは、そのまま読めて混ぜない
        for header in ["yolupainter-brush 1", "yolupainter-brush 2"] {
            let old = text(&user(1, Brush::default())).replacen("yolupainter-brush 1", header, 1);
            assert!(!decode(&old).unwrap().2.mix.is_active(), "{header}");
        }
        let bad = |from: &str, to: &str| decode(&v3.replace(from, to)).unwrap_err();
        assert!(
            matches!(bad("mix.mode=smear", "mix.mode=blend"), StoreError::BadValue(k) if k == "mix.mode")
        );
        assert!(
            matches!(bad("mix.ground=composite", "mix.ground=all"), StoreError::BadValue(k) if k == "mix.ground")
        );
        assert!(
            matches!(bad("mix.paint=0.625", "mix.paint=lots"), StoreError::BadValue(k) if k == "mix.paint")
        );
        assert!(matches!(
            bad("mix.paint=0.625", "mix.paint=1.5"),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            bad("mix.density=0.8", "mix.density=-0.1"),
            StoreError::Invalid(_)
        ));
        assert!(
            matches!(bad("mix.stretch=0.25", "mix.stretch=NaN"), StoreError::BadValue(k) if k == "mix.stretch")
        );
        assert!(matches!(
            bad("mix.paint.min=0.2", "mix.paint.min=2"),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            bad("mix.density.curve=0:0,0.5:0.75,1:1", "mix.density.curve=0:0;1:1"),
            StoreError::BadValue(k) if k == "mix.density.curve"
        ));
        assert!(matches!(
            bad(
                "mix.density.curve=0:0,0.5:0.75,1:1",
                "mix.density.curve=0:0,0.5:3,1:1"
            ),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            bad("mix.paint.pressure=1", "mix.paint.pressure=yes"),
            StoreError::BadValue(_)
        ));
        assert!(matches!(
            decode(&format!("{v3}mix.paint=0.5\n")).unwrap_err(),
            StoreError::DuplicateKey(_)
        ));
    }

    #[test]
    fn broken_files_are_refused_with_a_reason() {
        let ok = text(&user(1, Brush::default()));
        let bad = |edit: &dyn Fn(&str) -> String| decode(&edit(&ok)).unwrap_err();
        assert!(matches!(decode("").unwrap_err(), StoreError::NotABrush));
        assert!(matches!(
            decode("hello\n").unwrap_err(),
            StoreError::NotABrush
        ));
        assert!(matches!(
            decode("yolupainter-brush 6\nname=a\n").unwrap_err(),
            StoreError::NewerVersion(_)
        ));
        assert!(matches!(
            bad(&|t| format!("{t}radius=3\n")),
            StoreError::DuplicateKey(k) if k == "radius"
        ));
        assert!(matches!(
            bad(&|t| format!("{t}mystery=1\n")),
            StoreError::UnknownKey(k) if k == "mystery"
        ));
        assert!(matches!(
            bad(&|t| t.replace("radius=16", "radius=big")),
            StoreError::BadValue(k) if k == "radius"
        ));
        assert!(matches!(
            bad(&|t| t.replace("radius=16", "radius=NaN")),
            StoreError::BadValue(_)
        ));
        assert!(matches!(
            bad(&|t| t.replace("hardness=0.8", "hardness=7")),
            StoreError::Invalid(_)
        ));
        assert!(matches!(
            bad(&|t| t.replace("tip.image=none", "tip.image=photo")),
            StoreError::UnknownTip(n) if n == "photo"
        ));
        assert!(matches!(
            bad(&|t| format!("{t}garbage\n")),
            StoreError::Syntax(_)
        ));
        assert!(matches!(
            bad(&|t| t.replace("group=pen", "group=nowhere")),
            StoreError::BadValue(k) if k == "group"
        ));
        assert!(matches!(
            bad(&|t| t.replace("effect=paint", "effect=spray")),
            StoreError::BadValue(k) if k == "effect"
        ));
        assert!(matches!(
            bad(&|t| t.replace("pressure_size=1", "pressure_size=yes")),
            StoreError::BadValue(_)
        ));
    }

    fn custom(name: &str, seed: u8) -> Arc<BrushTip> {
        let alpha: Vec<u8> = (0..16u32).map(|i| (i as u8).wrapping_mul(seed)).collect();
        Arc::new(BrushTip::new(name, 4, 4, alpha).unwrap())
    }

    fn temp(name: &str) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/brush-store-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn carrying(assist: StrokeAssist) -> UserBrush {
        UserBrush {
            assist: Some(assist),
            ..user(1, Brush::default())
        }
    }

    #[test]
    fn a_brush_that_carries_start_end_and_stabilization_is_version_four_and_round_trips() {
        let assist = StrokeAssist {
            stabilizer: 6.0,
            taper_in: 1130.5,
            taper_out: 14.0,
            curve: false,
        };
        let u = carrying(assist);
        let t = text(&u);
        assert!(t.starts_with("yolupainter-brush 4\n"), "{t}");
        for line in [
            "assist.stabilizer=6\n",
            "assist.taper_in=1130.5\n",
            "assist.taper_out=14\n",
        ] {
            assert!(t.contains(line), "{t}");
        }
        let d = decode_user(&t, &mut |_| unreachable!()).unwrap();
        assert_eq!(d.assist, Some(assist));
        assert_eq!(
            d.brush,
            canonical(&Brush::default()),
            "ブラシの設定には手ぶれ補正・入り抜きを入れない"
        );
        // 持たないブラシ（全部 0 も持たない）は、今までと同じ版・同じ文
        let plain = text(&user(1, Brush::default()));
        assert!(
            plain.starts_with("yolupainter-brush 1\n") && !plain.contains("assist."),
            "{plain}"
        );
        assert_eq!(text(&carrying(StrokeAssist::default())), plain);
        assert_eq!(
            decode_user(&plain, &mut |_| unreachable!()).unwrap().assist,
            None
        );
        // 曲線の切り替えはブラシが持たない（描き手の設定）
        let with_curve = StrokeAssist {
            curve: true,
            ..assist
        };
        assert_eq!(text(&carrying(with_curve)), t);
        // 版 4 は、ほかの版の項目（筆圧の応え・色の混ぜ）も同じファイルに書く
        let both = UserBrush {
            assist: Some(assist),
            ..user(1, mixing_brush())
        };
        let t = text(&both);
        assert!(
            t.starts_with("yolupainter-brush 4\n") && t.contains("mix.mode="),
            "{t}"
        );
        let d = decode_user(&t, &mut |_| unreachable!()).unwrap();
        assert_eq!((d.assist, d.brush), (Some(assist), both.brush.clone()));
    }

    #[test]
    fn the_assist_items_belong_to_version_four_and_are_checked() {
        let t = text(&carrying(StrokeAssist {
            stabilizer: 3.0,
            taper_in: 20.0,
            taper_out: 0.0,
            curve: false,
        }));
        // 版 1〜3 のファイルにあれば知らない項目（今までの版の読み手と同じ断り方）
        for old in [
            "yolupainter-brush 1",
            "yolupainter-brush 2",
            "yolupainter-brush 3",
        ] {
            let as_old = t.replacen("yolupainter-brush 4", old, 1);
            assert!(
                matches!(decode(&as_old).unwrap_err(), StoreError::UnknownKey(k) if k.starts_with("assist.")),
                "{old}"
            );
        }
        // 版 6 は新しい形式として断る
        let newer = t.replacen("yolupainter-brush 4", "yolupainter-brush 6", 1);
        assert!(matches!(
            decode(&newer).unwrap_err(),
            StoreError::NewerVersion(_)
        ));
        // 範囲の外・有限でない値は、そのファイルを断る
        for bad in ["-1", "10001", "NaN", "inf", "x"] {
            let broken = t.replace("assist.taper_in=20", &format!("assist.taper_in={bad}"));
            assert!(
                matches!(
                    decode(&broken).unwrap_err(),
                    StoreError::Invalid(_) | StoreError::BadValue(_)
                ),
                "{bad}"
            );
        }
        // 項目が一部だけでも読める（無い項目は 0）。版 4 でも 1 つも無ければ持たない
        let some = "yolupainter-brush 4\nname=a\ngroup=pen\nassist.taper_out=9\n";
        let d = decode_user(some, &mut |_| unreachable!()).unwrap();
        assert_eq!(
            d.assist,
            Some(StrokeAssist {
                stabilizer: 0.0,
                taper_in: 0.0,
                taper_out: 9.0,
                curve: false
            })
        );
        let none = "yolupainter-brush 4\nname=a\ngroup=pen\n";
        assert_eq!(
            decode_user(none, &mut |_| unreachable!()).unwrap().assist,
            None
        );
    }

    fn anti_aliased(level: AntiAlias) -> Brush {
        let mut b = Brush::default();
        b.base.anti_alias = level;
        b.base.hardness = 1.0;
        b
    }

    #[test]
    fn an_anti_aliased_brush_is_version_five_and_round_trips() {
        for level in [AntiAlias::Weak, AntiAlias::Medium, AntiAlias::Strong] {
            let u = user(1, anti_aliased(level));
            let t = text(&u);
            assert!(t.starts_with("yolupainter-brush 5\n"), "{t}");
            assert!(t.contains(&format!("\nanti_alias={}\n", level.id())), "{t}");
            let d = decode_user(&t, &mut |_| unreachable!()).unwrap();
            assert_eq!(d.brush, u.brush);
            assert_eq!(d.brush.base.anti_alias, level);
        }
        // なし のブラシは今までと同じ版・同じ文（項目を書かない）
        let plain = text(&user(1, anti_aliased(AntiAlias::None)));
        assert!(
            plain.starts_with("yolupainter-brush 1\n") && !plain.contains("anti_alias"),
            "{plain}"
        );
        // 版 5 は、ほかの版の項目（色の混ぜ・入り抜き）も同じファイルに書く
        let mut mixing = mixing_brush();
        mixing.base.anti_alias = AntiAlias::Medium;
        let assist = StrokeAssist {
            stabilizer: 0.0,
            taper_in: 12.0,
            taper_out: 0.0,
            curve: false,
        };
        let both = UserBrush {
            assist: Some(assist),
            ..user(1, mixing)
        };
        let t = text(&both);
        assert!(
            t.starts_with("yolupainter-brush 5\n")
                && t.contains("mix.mode=")
                && t.contains("assist."),
            "{t}"
        );
        let d = decode_user(&t, &mut |_| unreachable!()).unwrap();
        assert_eq!((d.assist, d.brush), (Some(assist), both.brush.clone()));
    }

    #[test]
    fn the_anti_alias_item_belongs_to_version_five_and_is_checked() {
        let t = text(&user(1, anti_aliased(AntiAlias::Strong)));
        // 版 1〜4 のファイルにあれば知らない項目（今までの版の読み手と同じ断り方）
        for old in 1..=4 {
            let as_old = t.replacen(
                "yolupainter-brush 5",
                &format!("yolupainter-brush {old}"),
                1,
            );
            assert!(
                matches!(decode(&as_old).unwrap_err(), StoreError::UnknownKey(k) if k == "anti_alias"),
                "{old}"
            );
        }
        // 知らない名前はそのファイルを断る
        for bad in ["soft", "3", "", "Medium"] {
            let broken = t.replace("anti_alias=strong", &format!("anti_alias={bad}"));
            assert!(
                matches!(decode(&broken).unwrap_err(), StoreError::BadValue(k) if k == "anti_alias"),
                "{bad}"
            );
        }
        // 版 5 でも項目が無ければ なし。前の版のファイルも なし として読む
        let none = "yolupainter-brush 5\nname=a\ngroup=pen\n";
        assert_eq!(decode(none).unwrap().2.base.anti_alias, AntiAlias::None);
        let old = "yolupainter-brush 1\nname=a\ngroup=pen\nhardness=1\n";
        assert_eq!(decode(old).unwrap().2.base.anti_alias, AntiAlias::None);
        // 版 6 は新しい形式として断る
        let newer = t.replacen("yolupainter-brush 5", "yolupainter-brush 6", 1);
        assert!(matches!(
            decode(&newer).unwrap_err(),
            StoreError::NewerVersion(_)
        ));
    }

    #[test]
    fn a_carried_assist_is_saved_and_read_back_from_the_folder() {
        let dir = temp("assist");
        let store = BrushStore::new(dir.clone());
        let assist = StrokeAssist {
            stabilizer: 12.0,
            taper_in: 25.0,
            taper_out: 40.0,
            curve: false,
        };
        let mut u = carrying(assist);
        u.id = 3;
        store.save_brush(&u).unwrap();
        let mut plain = user(4, Brush::default());
        plain.name = "plain".into();
        store.save_brush(&plain).unwrap();
        let report = load_all(&dir);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        let by_id = |id| report.brushes.iter().find(|b| b.id == id).unwrap();
        assert_eq!(by_id(3).assist, Some(assist));
        assert_eq!(by_id(4).assist, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn imported_images_are_named_by_content_and_the_text_alone_cannot_stand_in_for_them() {
        let mut b = Brush::default();
        // 名前だけ組み込みと同じで中身が違う画像は、組み込みとして書かない
        b.tip.image = Some(custom("grain", 3));
        b.texture = Some(PaperTexture::new(custom("paper", 5), 0.5));
        b.dual = Some(DualBrush {
            tip: Some(custom("dual", 7)),
            ..DualBrush::default()
        });
        let encoded = encode(&user(1, b.clone())).unwrap();
        assert_eq!(encoded.images.len(), 3);
        for (hash, tip) in &encoded.images {
            assert!(images::is_fingerprint(hash));
            assert_eq!(*hash, images::fingerprint(tip));
            assert!(
                encoded.text.contains(&format!("img:{hash}")),
                "{}",
                encoded.text
            );
        }
        assert!(
            !encoded.text.contains("tip.image=grain"),
            "{}",
            encoded.text
        );
        // 画像を渡さずに読むと、知らない画像として断る（画像のファイルが無いブラシは通さない）
        assert!(
            matches!(decode(&encoded.text).unwrap_err(), StoreError::UnknownTip(n) if n.starts_with("img:"))
        );
        // 同じ画像を 2 か所に使っても 1 枚
        let mut again = Brush::default();
        again.tip.image = Some(custom("same", 9));
        again.texture = Some(PaperTexture::new(custom("same", 9), 0.5));
        assert_eq!(encode(&user(2, again)).unwrap().images.len(), 1);
    }

    #[test]
    fn imported_brushes_are_saved_with_their_images_and_read_back_equal_with_shared_images() {
        let dir = temp("images");
        let store = BrushStore::new(dir.clone());
        let tip = custom("tip", 3);
        let mut a = Brush::default();
        a.tip.image = Some(tip.clone());
        a.texture = Some(PaperTexture::new(custom("paper", 5), 0.5));
        let mut hose = Brush::default();
        hose.tip.images = vec![custom("h1", 11), custom("h2", 13), tip.clone()];
        hose.tip.selection = TipSelection::Sequential;
        let meta = ImportMeta::new(
            "Photoshop ABR v10".into(),
            true,
            vec![Gap::WetEdges, Gap::ColorTip, Gap::WetEdges],
        );
        let mut one = user(1, a);
        one.group = Group::Imported;
        one.import = Some(meta.clone());
        let two = user(2, hose);
        store.save_brush(&one).unwrap();
        store.save_brush(&two).unwrap();
        let report = load_all(&dir);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.brushes.len(), 2);
        assert_eq!(report.brushes[0].brush, one.brush);
        assert_eq!(report.brushes[1].brush, two.brush);
        assert_eq!(report.brushes[0].group, Group::Imported);
        let got = report.brushes[0].import.as_ref().unwrap();
        assert_eq!(got.source, "Photoshop ABR v10");
        assert!(got.pattern);
        assert_eq!(
            got.gaps,
            [Gap::ColorTip, Gap::WetEdges],
            "並び順で重ならない"
        );
        assert_eq!(report.brushes[1].import, None);
        // 2 つのブラシが使う同じ画像は読んだあとも 1 つを共有する
        let first = report.brushes[0].brush.tip.image.as_ref().unwrap();
        let shared = &report.brushes[1].brush.tip.images[2];
        assert!(Arc::ptr_eq(first, shared));
        // 画像のファイルは 5 枚（tip・paper・h1・h2。tip は共有）と、先に置いた一時ファイルが残らないこと
        let files: Vec<String> = std::fs::read_dir(store.images_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files.len(), 4, "{files:?}");
        assert!(files.iter().all(|f| f.ends_with(".png")), "{files:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn deleting_a_brush_removes_only_the_images_nobody_else_uses() {
        let dir = temp("gc");
        let store = BrushStore::new(dir.clone());
        let shared = custom("shared", 3);
        let mut a = Brush::default();
        a.tip.image = Some(shared.clone());
        a.texture = Some(PaperTexture::new(custom("only-a", 5), 0.5));
        let mut b = Brush::default();
        b.tip.image = Some(shared.clone());
        store.save_brush(&user(1, a)).unwrap();
        store.save_brush(&user(2, b)).unwrap();
        let count = || std::fs::read_dir(store.images_dir()).unwrap().count();
        assert_eq!(count(), 2);
        store.delete_brush(1).unwrap();
        assert_eq!(count(), 1, "a だけの画像は消え、共有の画像は残る");
        assert!(load_all(&dir).problems.is_empty());
        // 読めないファイルが画像を指しているかもしれないときは、何も消さない（新しい版のファイルも）
        let hash = images::fingerprint(&shared);
        std::fs::write(
            store.path_of(3),
            format!("yolupainter-brush 9\ntip.image=img:{hash}\n"),
        )
        .unwrap();
        store.delete_brush(2).unwrap();
        assert_eq!(count(), 1, "新しい版のファイルが指している画像は残す");
        store.delete_brush(3).unwrap();
        assert_eq!(count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_damaged_or_swapped_image_file_makes_that_brush_unreadable_and_nothing_else() {
        let dir = temp("damaged");
        let store = BrushStore::new(dir.clone());
        let (tip_a, tip_b) = (custom("a", 3), custom("b", 5));
        let mut a = Brush::default();
        a.tip.image = Some(tip_a.clone());
        let mut b = Brush::default();
        b.tip.image = Some(tip_b.clone());
        store.save_brush(&user(1, a)).unwrap();
        store.save_brush(&user(2, b)).unwrap();
        store.save_brush(&user(3, Brush::default())).unwrap();
        // a の画像に b の画像の中身を置く（PNG としては読めるが、名前（指紋）と中身が合わない）
        let (hash_a, hash_b) = (images::fingerprint(&tip_a), images::fingerprint(&tip_b));
        std::fs::copy(store.image_path(&hash_b), store.image_path(&hash_a)).unwrap();
        let report = load_all(&dir);
        let ids: Vec<u32> = report.brushes.iter().map(|b| b.id).collect();
        assert_eq!(ids, [2, 3]);
        assert!(
            matches!(report.problems[0].reason, StoreError::BadImage(_)),
            "{:?}",
            report.problems
        );
        // 画像のファイルが無いときは、知らない画像
        std::fs::remove_file(store.image_path(&hash_b)).unwrap();
        let report = load_all(&dir);
        assert!(report.problems.iter().all(|p| matches!(
            p.reason,
            StoreError::UnknownTip(_) | StoreError::BadImage(_)
        )));
        for p in &report.problems {
            assert!(!p.describe(Lang::Ja).is_empty() && !p.describe(Lang::En).is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_gap_names_are_skipped_and_a_bad_pattern_flag_is_refused() {
        let u = UserBrush {
            import: Some(ImportMeta::new("GIMP GBR".into(), false, vec![Gap::Noise])),
            ..user(1, Brush::default())
        };
        let ok = text(&u);
        let with_unknown = ok.replace("import.gaps=noise", "import.gaps=noise,from-the-future");
        let (_, _, _) = decode(&with_unknown).unwrap();
        let d = decode_user(&with_unknown, &mut |_| unreachable!()).unwrap();
        assert_eq!(d.import.unwrap().gaps, [Gap::Noise]);
        assert!(matches!(
            decode(&format!("{ok}import.pattern=yes\n")).unwrap_err(),
            StoreError::DuplicateKey(_) | StoreError::BadValue(_)
        ));
        // 取り込みの印が無いブラシのファイルは、これまでと同じ文（古いファイルも新しい版で変わらない）
        assert!(!text(&user(2, Brush::default())).contains("import."));
    }

    #[test]
    fn carried_over_items_are_saved_by_name_and_unknown_names_are_skipped() {
        use yolu_io::brushes::SutMapped;
        let mapped_of = |text: &str| {
            decode_user(text, &mut |_| unreachable!())
                .unwrap()
                .import
                .unwrap()
                .mapped
        };
        let meta = ImportMeta::new("CLIP STUDIO .sut".into(), false, vec![Gap::Spray])
            .with_mapped(vec![SutMapped::Tilt, SutMapped::StartEnd, SutMapped::Tilt]);
        assert_eq!(
            meta.mapped,
            [SutMapped::Tilt, SutMapped::StartEnd],
            "並び順で重ならない"
        );
        let u = UserBrush {
            import: Some(meta.clone()),
            ..user(1, Brush::default())
        };
        let t = text(&u);
        assert!(t.contains("import.mapped=tilt,start-end\n"), "{t}");
        // 取り込みの項目は版を上げない（版は、描き方を変える項目を使うブラシだけが上げる）
        assert!(t.starts_with("yolupainter-brush 1\n"), "{t}");
        let back = decode_user(&t, &mut |_| unreachable!()).unwrap();
        assert_eq!(back.import, Some(meta));
        // 知らない名前（新しい版が足した項目）は読み飛ばす
        let future = t.replace(
            "import.mapped=tilt,start-end",
            "import.mapped=tilt,from-the-future,start-end",
        );
        assert_eq!(mapped_of(&future), [SutMapped::Tilt, SutMapped::StartEnd]);
        // 写した項目の欄が無い古いファイルは、空（今のブラシの設定から導いて足さない）
        let old = t.replace("import.mapped=tilt,start-end\n", "");
        assert_eq!(mapped_of(&old), []);
        // 写した項目だけのファイルも、取り込みの印として読む
        let only = format!(
            "{}import.mapped=texture\n",
            text(&user(2, Brush::default()))
        );
        assert_eq!(mapped_of(&only), [SutMapped::Texture]);
        // 全項目が名前で往復する
        let all = ImportMeta::new(String::new(), false, vec![])
            .with_mapped(super::super::gaps::MAPPED_ALL.to_vec());
        let u = UserBrush {
            import: Some(all.clone()),
            ..user(3, Brush::default())
        };
        assert_eq!(mapped_of(&text(&u)), all.mapped);
    }

    #[test]
    fn krita_tips_are_saved_by_id_once_the_set_is_loaded_and_read_back_equal() {
        let krita = krita();
        let single = krita
            .brushes
            .iter()
            .find(|b| b.brush.tip.image.is_some())
            .expect("1 枚の筆先");
        let hose = krita
            .brushes
            .iter()
            .find(|b| b.brush.tip.images.len() > 1)
            .expect("ホース");
        let mut a = Brush::default();
        a.tip.image = single.brush.tip.image.clone();
        let mut b = Brush::default();
        b.tip.images = hose.brush.tip.images.clone();
        b.tip.selection = hose.brush.tip.selection;
        for (n, brush, id) in [(1, a, &single.id), (2, b, &hose.id)] {
            let encoded = encode(&user(n, brush.clone())).unwrap();
            assert!(
                encoded.images.is_empty(),
                "同梱の筆先は画像のファイルを置かない"
            );
            assert!(encoded.text.contains(id.as_str()), "{}", encoded.text);
            let (_, _, back) = decode(&encoded.text).unwrap();
            assert_eq!(back, canonical(&brush));
        }
        // 1 枚の筆先の欄に、ホースの札は入らない（値が読めない）
        let mut bad = text(&user(3, Brush::default()));
        bad = bad.replace("tip.image=none", &format!("tip.image={}", hose.id));
        assert!(matches!(decode(&bad).unwrap_err(), StoreError::BadValue(k) if k == "tip.image"));
        let unknown = bad.replace(&hose.id, "bundled:krita4/no-such-file.png");
        assert!(matches!(
            decode(&unknown).unwrap_err(),
            StoreError::UnknownTip(_)
        ));
    }

    #[test]
    fn replacing_keeps_the_old_file_when_the_replace_fails_and_loading_skips_bad_files() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/brush-store-tests")
            .join(format!("a-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = BrushStore::new(dir.clone());
        // 空のフォルダ・無いフォルダは空
        assert!(load_all(&dir).brushes.is_empty());
        assert_eq!(load_all(&dir).max_file_id, None);
        let mut first = user(1, Brush::default());
        store.save_brush(&first).unwrap();
        let mut second_brush = Brush::default();
        second_brush.base.radius = 40.0;
        let second = user(2, second_brush);
        store.save_brush(&second).unwrap();
        store
            .save_order(&["u:2".into(), "u:1".into(), "b:pencil".into()])
            .unwrap();
        // 壊れたファイル・知らない名前のファイル・一時ファイルが混ざっても、読めるものは読む
        std::fs::write(dir.join("brush-00000003.ylbrush"), "not a brush").unwrap();
        std::fs::write(dir.join("brush-00000004.ylbrush"), vec![b'a'; 70_000]).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        std::fs::write(dir.join("brush-00000001.ylbrush.99.pending"), "half").unwrap();
        let report = load_all(&dir);
        let ids: Vec<u32> = report.brushes.iter().map(|b| b.id).collect();
        assert_eq!(ids, [1, 2]);
        assert_eq!(report.brushes[1], second);
        let mut problems: Vec<(&str, bool)> = report
            .problems
            .iter()
            .map(|p| (p.file.as_str(), matches!(p.reason, StoreError::TooLarge)))
            .collect();
        problems.sort();
        assert_eq!(
            problems,
            [
                ("brush-00000003.ylbrush", false),
                ("brush-00000004.ylbrush", true)
            ]
        );
        for p in &report.problems {
            assert!(!p.describe(Lang::Ja).is_empty() && !p.describe(Lang::En).is_empty());
        }
        assert_eq!(report.order, ["u:2", "u:1", "b:pencil"]);
        // 読めなかったファイルの番号（4）も最大に入る。名前の形でないファイルと一時ファイルは入らない
        assert_eq!(report.max_file_id, Some(4));
        // 読めなかったファイルは「読み込まなかった番号」に入る（読めた物と、フォルダに無い番号は入らない）
        assert_eq!(report.unloaded, [3, 4]);
        assert!(store.is_taken(3) && store.is_taken(4) && !store.is_taken(5));
        // 置換できないとき、元のファイルは変わらず、一時ファイルも残らない
        let before = std::fs::read(store.path_of(1)).unwrap();
        let files = std::fs::read_dir(&dir).unwrap().count();
        first.brush.base.radius = 99.0;
        assert!(yolu_io::atomic::failing(|| store.save_brush(&first)).is_err());
        assert_eq!(std::fs::read(store.path_of(1)).unwrap(), before);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), files);
        store.save_brush(&first).unwrap();
        assert_ne!(std::fs::read(store.path_of(1)).unwrap(), before);
        // 消す
        store.delete_brush(1).unwrap();
        store.delete_brush(1).unwrap();
        assert!(!store.path_of(1).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn too_many_files_are_read_only_up_to_the_limit_and_reported() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/brush-store-tests")
            .join(format!("c-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for id in 1..=(MAX_FILES as u32 + 6) {
            std::fs::write(dir.join(file_name(id)), "x").unwrap();
        }
        let report = load_all(&dir);
        assert_eq!(report.brushes.len(), 0);
        // 読まなかった番号も、次に付ける番号の根拠に入る
        assert_eq!(report.max_file_id, Some(MAX_FILES as u32 + 6));
        assert_eq!(
            report
                .problems
                .iter()
                .filter(|p| matches!(p.reason, StoreError::TooMany))
                .count(),
            1
        );
        // 読んだ（読もうとした）のは番号の小さい順に MAX_FILES 個。残りには触れない
        assert_eq!(report.problems.len(), MAX_FILES + 1);
        assert!(!report
            .problems
            .iter()
            .any(|p| p.file == file_name(MAX_FILES as u32 + 1)));
        // 上限で読まなかった番号も「読み込まなかった番号」に入る
        assert_eq!(report.unloaded.len(), MAX_FILES + 6);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_newer_format_is_left_untouched_and_reported() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/brush-store-tests")
            .join(format!("b-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brush-00000005.ylbrush");
        std::fs::write(&path, "yolupainter-brush 6\nname=future\n").unwrap();
        let report = load_all(&dir);
        assert!(report.brushes.is_empty());
        assert!(matches!(
            report.problems[0].reason,
            StoreError::NewerVersion(_)
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "yolupainter-brush 6\nname=future\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
