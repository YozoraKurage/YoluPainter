//! 利用者のブラシの保存（設定のフォルダの `brushes/`）。ブラシ 1 つが 1 ファイル（`brush-<番号>.ylbrush`）、並びは `order.conf`。
//!
//! 形式は 1 行目が `yolupainter-brush 1`、あとは `key=value` の行（UTF-8、64 KiB まで）。数は Rust の表記のまま書き（読み戻しても
//! 同じ値）、筆先・質感の画像は組み込みの名前で持つ（取り込んだ画像は保存できない）。知らない項目・重なった項目・範囲を外れた値は
//! そのファイルを読み飛ばして理由を残す（ほかのファイルは読む）。版が新しいファイルは触らずに読み飛ばす。
//! 書き込みは一時ファイルへ書いて読み戻して確かめてから、最後の 1 回の置換で確定する（置換に失敗したら元のファイルは変わらない）。
//! 名前はファイルの中だけに持ち、ファイル名には使わない。

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::{canonical, Group, UserBrush, MAX_NAME_CHARS};
use crate::engine::{
    Brush, BrushEffect, CoreError, DVec2, DualBrush, DualBrushMode, PaperTexture, TextureMode,
};
use crate::lang::Lang;
use yolu_core::brush::{builtin_tip, TipSelection};

pub const HEADER: &str = "yolupainter-brush 1";
const EXTENSION: &str = "ylbrush";
const ORDER_FILE: &str = "order.conf";
/// 1 ファイルの大きさの上限。
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// 読むファイルの数の上限（これより多い分は読み飛ばす）。
const MAX_FILES: usize = 1024;

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
    /// 取り込んだ画像の筆先・質感は保存できない。
    CustomImage,
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
            StoreError::UnknownKey(k) => {
                lang.pick(format!("知らない項目: {k}"), format!("Unknown item: {k}"))
            }
            StoreError::DuplicateKey(k) => lang.pick(
                format!("項目が重なっています: {k}"),
                format!("Repeated item: {k}"),
            ),
            StoreError::BadValue(k) => lang.pick(
                format!("値が読めません: {k}"),
                format!("Invalid value: {k}"),
            ),
            StoreError::UnknownTip(n) => {
                lang.pick(format!("知らない画像: {n}"), format!("Unknown image: {n}"))
            }
            StoreError::Invalid(e) => lang.core_error(e),
            StoreError::CustomImage => lang
                .pick(
                    "取り込んだ画像の筆先は保存できません",
                    "Imported tip images cannot be saved",
                )
                .into(),
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

/// 組み込みの筆先の名前（取り込んだ画像・名前だけ同じで中身が違う画像は保存できない）。
fn tip_name(tip: &yolu_core::BrushTip) -> Result<&str, StoreError> {
    match builtin_tip(tip.name()) {
        Some(b) if *b == *tip => Ok(tip.name()),
        _ => Err(StoreError::CustomImage),
    }
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

/// ブラシを書き出す（`brush` は正規の形でなくても、画面が持たない項目は書かない）。
pub fn encode(user: &UserBrush) -> Result<String, StoreError> {
    let b = canonical(&user.brush);
    let mut w = Writer(format!("{HEADER}\n"));
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
    let t = &b.tip;
    match &t.image {
        Some(image) => w.line("tip.image", tip_name(image)?),
        None => w.line("tip.image", "none"),
    }
    if t.images.is_empty() {
        w.line("tip.images", "none");
    } else {
        let names: Result<Vec<&str>, StoreError> = t.images.iter().map(|i| tip_name(i)).collect();
        w.line("tip.images", names?.join(","));
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
        w.line("texture", tip_name(&tex.image)?);
        w.line("texture.depth", tex.depth);
        w.line("texture.scale", tex.scale);
        w.line("texture.mode", mode_id(tex.mode));
    }
    if let Some(d) = &b.dual {
        w.line("dual", 1);
        match &d.tip {
            Some(image) => w.line("dual.tip", tip_name(image)?),
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
    Ok(w.0)
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
    fn tip(
        &mut self,
        key: &str,
    ) -> Result<Option<std::sync::Arc<yolu_core::BrushTip>>, StoreError> {
        match self.take(key) {
            None => Ok(None),
            Some(name) if name == "none" => Ok(None),
            Some(name) => builtin_tip(&name)
                .map(Some)
                .ok_or(StoreError::UnknownTip(name)),
        }
    }
}

fn texture_mode(id: &str) -> Option<TextureMode> {
    TextureMode::ALL.into_iter().find(|m| mode_id(*m) == id)
}

fn dual_mode(id: &str) -> Option<DualBrushMode> {
    DualBrushMode::ALL
        .into_iter()
        .find(|m| dual_mode_id(*m) == id)
}

/// ファイルの中身から、名前・グループ・ブラシ（正規の形）を読む。
pub fn decode(text: &str) -> Result<(String, Group, Brush), StoreError> {
    let mut lines = text.lines();
    match lines.next() {
        Some(HEADER) => {}
        Some(first) if first.starts_with("yolupainter-brush ") => {
            return Err(StoreError::NewerVersion(first.to_owned()))
        }
        _ => return Err(StoreError::NotABrush),
    }
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
    b.tip.image = r.tip("tip.image")?;
    if let Some(list) = r.take("tip.images") {
        if list != "none" {
            for name in list.split(',') {
                b.tip.images.push(
                    builtin_tip(name).ok_or_else(|| StoreError::UnknownTip(name.to_owned()))?,
                );
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
    if let Some(image) = r.tip("texture")? {
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
        dual.tip = r.tip("dual.tip")?;
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
    if let Some(key) = r.map.keys().min() {
        return Err(StoreError::UnknownKey(key.clone()));
    }
    b.validate().map_err(StoreError::Invalid)?;
    Ok((name, group, canonical(&b)))
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

/// 一時ファイルに書いて同期し、`verify` が読み戻しを確かめたら `path` へ置き換える。失敗したら一時ファイルを消す。
fn replace_file(
    path: &Path,
    text: &str,
    verify: impl FnOnce(&str) -> bool,
) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "brush directory missing"))?;
    std::fs::create_dir_all(parent)?;
    let pending = path.with_extension(format!(
        "{}.{}.pending",
        path.extension().and_then(|e| e.to_str()).unwrap_or("tmp"),
        std::process::id()
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&pending)?;
    let result = (|| -> Result<(), StoreError> {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        if !verify(&read_text(&pending)?) {
            return Err(StoreError::Mismatch);
        }
        std::fs::rename(&pending, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&pending);
    }
    result
}

/// 設定のフォルダの `brushes/`。
#[derive(Clone, Debug)]
pub struct BrushStore {
    dir: PathBuf,
}

impl BrushStore {
    pub fn new(dir: PathBuf) -> BrushStore {
        BrushStore { dir }
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

    pub fn save_brush(&self, user: &UserBrush) -> Result<(), StoreError> {
        let text = encode(user)?;
        let expect = (
            super::clean_name(&user.name).unwrap_or_default(),
            user.group,
            canonical(&user.brush),
        );
        replace_file(&self.path_of(user.id), &text, |read| {
            decode(read).is_ok_and(|got| got == expect)
        })
    }

    pub fn delete_brush(&self, id: u32) -> Result<(), StoreError> {
        match std::fs::remove_file(self.path_of(id)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
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
    for (id, name, _) in files {
        let loaded = read_text(&dir.join(&name)).and_then(|text| decode(&text));
        match loaded {
            Ok((display, group, brush)) => report.brushes.push(UserBrush {
                id,
                name: display,
                group,
                brush,
            }),
            Err(reason) => report.problems.push(Problem { file: name, reason }),
        }
    }
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

    fn user(id: u32, brush: Brush) -> UserBrush {
        UserBrush {
            id,
            name: format!("テスト {id}"),
            group: Group::Pen,
            brush: canonical(&brush),
        }
    }

    #[test]
    fn every_builtin_round_trips_through_the_text_format() {
        for b in super::super::builtin::all() {
            let u = UserBrush {
                id: 1,
                name: "x".into(),
                group: b.group,
                brush: b.brush.clone(),
            };
            let text = encode(&u).unwrap_or_else(|e| panic!("{}: {e:?}", b.id));
            let (name, group, brush) =
                decode(&text).unwrap_or_else(|e| panic!("{}: {e:?}\n{text}", b.id));
            assert_eq!((name.as_str(), group), ("x", b.group), "{}", b.id);
            assert_eq!(brush, b.brush, "{}", b.id);
        }
        for p in m2::presets() {
            let u = user(2, p.brush.clone());
            assert_eq!(decode(&encode(&u).unwrap()).unwrap().2, u.brush, "{}", p.id);
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
        let text = encode(&u).unwrap();
        let (_, _, back) = decode(&text).unwrap();
        assert_eq!(back, u.brush);
        // 書き直しても同じ文
        let again = encode(&UserBrush {
            brush: back,
            ..u.clone()
        })
        .unwrap();
        assert_eq!(again, text);
        assert!(text.starts_with("yolupainter-brush 1\n"), "{text}");
    }

    #[test]
    fn broken_files_are_refused_with_a_reason() {
        let ok = encode(&user(1, Brush::default())).unwrap();
        let bad = |edit: &dyn Fn(&str) -> String| decode(&edit(&ok)).unwrap_err();
        assert!(matches!(decode("").unwrap_err(), StoreError::NotABrush));
        assert!(matches!(
            decode("hello\n").unwrap_err(),
            StoreError::NotABrush
        ));
        assert!(matches!(
            decode("yolupainter-brush 2\nname=a\n").unwrap_err(),
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

    #[test]
    fn imported_tip_images_are_not_saved() {
        let custom = std::sync::Arc::new(
            yolu_core::BrushTip::new("grain", 2, 2, vec![0, 255, 255, 0]).unwrap(),
        );
        let mut b = Brush::default();
        b.tip.image = Some(custom);
        assert!(matches!(
            encode(&user(1, b)).unwrap_err(),
            StoreError::CustomImage
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
        assert!(store.is_taken(3) && store.is_taken(4) && !store.is_taken(5));
        // 一時ファイルの名前が塞がっていて置換できないとき、元のファイルは変わらない
        let before = std::fs::read(store.path_of(1)).unwrap();
        let pending = store
            .path_of(1)
            .with_extension(format!("ylbrush.{}.pending", std::process::id()));
        std::fs::write(&pending, "busy").unwrap();
        first.brush.base.radius = 99.0;
        assert!(store.save_brush(&first).is_err());
        assert_eq!(std::fs::read(store.path_of(1)).unwrap(), before);
        assert_eq!(std::fs::read(&pending).unwrap(), b"busy");
        std::fs::remove_file(&pending).unwrap();
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
            report.problems.iter().filter(|p| matches!(p.reason, StoreError::TooMany)).count(),
            1
        );
        // 読んだ（読もうとした）のは番号の小さい順に MAX_FILES 個。残りには触れない
        assert_eq!(report.problems.len(), MAX_FILES + 1);
        assert!(!report
            .problems
            .iter()
            .any(|p| p.file == file_name(MAX_FILES as u32 + 1)));
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
        std::fs::write(&path, "yolupainter-brush 3\nname=future\n").unwrap();
        let report = load_all(&dir);
        assert!(report.brushes.is_empty());
        assert!(matches!(
            report.problems[0].reason,
            StoreError::NewerVersion(_)
        ));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "yolupainter-brush 3\nname=future\n"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
