//! 利用者のグラデーションセットの保存（設定のフォルダの `gradients/` の `user.ylgrad`）。サブツールの `.ylsubtool` と同じ流儀の形で、
//! 1 行目が `yolupainter-gradients 1`、あとは `key=value` の行（UTF-8、8 MiB まで、空行は読み飛ばす）。
//! ```text
//! yolupainter-gradients 1
//! ramp.1.name=夕空
//! ramp.1.colors=0:1a2040:0.5;0.5:cc5060:0.3;1:ffd080:0.5
//! ramp.1.opacities=0:1:0.5;1:1:0.5
//! ramp.1.curve=0:0;0.5:0.6;1:1
//! ramp.1.segment.0=0:0;0.3:0.8;1:1
//! ```
//! `colors` は `位置:RRGGBB:中点` を `;` でつなぎ、`opacities` は `位置:不透明度:中点`、`curve`（値のカーブ。無ければ直線）と
//! `segment.<区間>`（その区間の混合率曲線。無ければ中点）は `x:y` の列。数は Rust の表記のまま書く（読み戻しても同じ値）。混色モード・輝度の補正は
//! セットに入れない（グラデーションマップの設定で、セットは分岐点の並びだけ）。番号は 1 から、セットは番号の順。知らない項目・重なった項目・
//! 範囲を外れた値・名前や色の無いもの・多すぎるものは、ファイルごと読み飛ばして理由を残す。版が新しいファイルは触らずに読み飛ばす。
//! 書き込みは、一時ファイルへ書いて読み戻して確かめてから、最後の 1 回の置換で確定する（途中で落ちても前の版が残る）。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use yolu_core::curve::{Curve, CurvePoint};
use yolu_core::generator::{ColorStop, OpacityStop, Ramp};
use yolu_core::Rgba8;

use super::UserRamp;
use crate::brushes::clean_name;
use crate::lang::Lang;

pub const HEADER: &str = "yolupainter-gradients 1";
pub const FILE_NAME: &str = "user.ylgrad";
/// 1 ファイルの大きさの上限。数の上限（`MAX_USER`）まで、どんなランプ（分岐点 32・カーブの点 16・区間の曲線 31 本）でも収まる大きさ
/// （試験が、いちばん長い書き方で確かめる）。数の上限に先に当たるので、足したセットだけがメモリに残って保存できない、ということが起きない。
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// 利用者のセットの数の上限。
pub const MAX_USER: usize = 128;

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    TooLarge,
    /// 1 行目がこの形式でない。
    NotGradients,
    /// 今の版より新しい形式。
    NewerVersion(String),
    /// `key=value` でない行（行番号）。
    Syntax(usize),
    UnknownKey(String),
    DuplicateKey(String),
    BadValue(String),
    TooMany,
    /// 書いたファイルを読み戻したら、書いたものと違った。
    Mismatch,
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
            StoreError::NotGradients => lang
                .pick(
                    "グラデーションセットのファイルではありません",
                    "Not a gradient set file",
                )
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
            StoreError::TooMany => lang.pick("セットが多すぎます", "Too many gradients").into(),
            StoreError::Mismatch => lang
                .pick(
                    "書いた内容を読み戻せませんでした",
                    "The written file did not read back the same",
                )
                .into(),
        }
    }
}

fn pairs(points: &[CurvePoint]) -> String {
    points
        .iter()
        .map(|p| format!("{}:{}", p.x, p.y))
        .collect::<Vec<_>>()
        .join(";")
}

/// 利用者のセットを書く文にする。
pub fn encode(sets: &[UserRamp]) -> String {
    let mut text = format!("{HEADER}\n");
    for (i, set) in sets.iter().enumerate() {
        let n = i + 1;
        let r = &set.ramp;
        text.push_str(&format!(
            "ramp.{n}.name={}\n",
            clean_name(&set.name).unwrap_or_default()
        ));
        let colors: Vec<String> = r
            .colors()
            .iter()
            .map(|c| {
                format!(
                    "{}:{:02x}{:02x}{:02x}:{}",
                    c.position, c.color.r, c.color.g, c.color.b, c.midpoint
                )
            })
            .collect();
        text.push_str(&format!("ramp.{n}.colors={}\n", colors.join(";")));
        let opacities: Vec<String> = r
            .opacities()
            .iter()
            .map(|o| format!("{}:{}:{}", o.position, o.opacity, o.midpoint))
            .collect();
        text.push_str(&format!("ramp.{n}.opacities={}\n", opacities.join(";")));
        if !r.value_curve().is_identity() {
            text.push_str(&format!("ramp.{n}.curve={}\n", pairs(r.curve())));
        }
        for (k, curve) in r.segment_curves().iter().enumerate() {
            if let Some(curve) = curve {
                text.push_str(&format!("ramp.{n}.segment.{k}={}\n", pairs(curve.points())));
            }
        }
    }
    text
}

fn float(text: &str, key: &str) -> Result<f64, StoreError> {
    text.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .ok_or_else(|| StoreError::BadValue(key.to_owned()))
}

fn curve_of(text: &str, key: &str) -> Result<Curve, StoreError> {
    let bad = || StoreError::BadValue(key.to_owned());
    let mut points = Vec::new();
    for pair in text.split(';') {
        let (x, y) = pair.split_once(':').ok_or_else(bad)?;
        points.push(CurvePoint {
            x: float(x, key)?,
            y: float(y, key)?,
        });
    }
    Curve::new(points).map_err(|_| bad())
}

#[derive(Default)]
struct Pending {
    name: Option<String>,
    colors: Option<Vec<ColorStop>>,
    opacities: Option<Vec<OpacityStop>>,
    curve: Option<Curve>,
    segments: BTreeMap<usize, Curve>,
}

/// 文を読む。
pub fn decode(text: &str) -> Result<Vec<UserRamp>, StoreError> {
    let mut lines = text.lines().enumerate();
    let first = lines.next().map(|(_, l)| l.trim_end()).unwrap_or("");
    if first != HEADER {
        return match first.strip_prefix("yolupainter-gradients ") {
            Some(version) => Err(StoreError::NewerVersion(version.trim().to_owned())),
            None => Err(StoreError::NotGradients),
        };
    }
    let mut found: BTreeMap<u32, Pending> = BTreeMap::new();
    for (index, line) in lines {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(StoreError::Syntax(index + 1))?;
        let unknown = || StoreError::UnknownKey(key.to_owned());
        let rest = key.strip_prefix("ramp.").ok_or_else(unknown)?;
        let (number, field) = rest.split_once('.').ok_or_else(unknown)?;
        let id: u32 = number
            .parse()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(unknown)?;
        if !found.contains_key(&id) && found.len() >= MAX_USER {
            return Err(StoreError::TooMany);
        }
        let entry = found.entry(id).or_default();
        let bad = || StoreError::BadValue(key.to_owned());
        let dup = || StoreError::DuplicateKey(key.to_owned());
        match field {
            "name" => {
                if entry.name.is_some() {
                    return Err(dup());
                }
                entry.name = Some(clean_name(value).ok_or_else(bad)?);
            }
            "colors" => {
                if entry.colors.is_some() {
                    return Err(dup());
                }
                let mut list = Vec::new();
                for part in value.split(';') {
                    let mut it = part.split(':');
                    let (p, c, m) = match (it.next(), it.next(), it.next(), it.next()) {
                        (Some(p), Some(c), Some(m), None) => (p, c, m),
                        _ => return Err(bad()),
                    };
                    let hex = u32::from_str_radix(c, 16)
                        .ok()
                        .filter(|_| c.len() == 6 && c.bytes().all(|b| b.is_ascii_hexdigit()))
                        .ok_or_else(bad)?;
                    list.push(ColorStop {
                        position: float(p, key)?,
                        color: Rgba8::new((hex >> 16) as u8, (hex >> 8) as u8, hex as u8, 255),
                        midpoint: float(m, key)?,
                    });
                }
                entry.colors = Some(list);
            }
            "opacities" => {
                if entry.opacities.is_some() {
                    return Err(dup());
                }
                let mut list = Vec::new();
                for part in value.split(';') {
                    let mut it = part.split(':');
                    let (p, o, m) = match (it.next(), it.next(), it.next(), it.next()) {
                        (Some(p), Some(o), Some(m), None) => (p, o, m),
                        _ => return Err(bad()),
                    };
                    list.push(OpacityStop {
                        position: float(p, key)?,
                        opacity: float(o, key)?,
                        midpoint: float(m, key)?,
                    });
                }
                entry.opacities = Some(list);
            }
            "curve" => {
                if entry.curve.is_some() {
                    return Err(dup());
                }
                entry.curve = Some(curve_of(value, key)?);
            }
            other => {
                let k: usize = other
                    .strip_prefix("segment.")
                    .and_then(|k| k.parse().ok())
                    .ok_or_else(unknown)?;
                if entry.segments.contains_key(&k) {
                    return Err(dup());
                }
                entry.segments.insert(k, curve_of(value, key)?);
            }
        }
    }
    found
        .into_iter()
        .map(|(id, p)| {
            let missing = |what: &str| StoreError::BadValue(format!("ramp.{id}.{what}"));
            let name = p.name.ok_or_else(|| missing("name"))?;
            let colors = p.colors.ok_or_else(|| missing("colors"))?;
            let opacities = p.opacities.ok_or_else(|| missing("opacities"))?;
            let curve = p.curve.map(|c| c.points().to_vec());
            let count = colors.len();
            let mut ramp = Ramp::new(colors, opacities, curve).map_err(|_| missing("colors"))?;
            if !p.segments.is_empty() {
                let mut list = vec![None; count.saturating_sub(1)];
                for (k, curve) in p.segments {
                    *list
                        .get_mut(k)
                        .ok_or_else(|| missing(&format!("segment.{k}")))? = Some(curve);
                }
                ramp = ramp
                    .with_segment_curves(list)
                    .map_err(|_| missing("segment"))?;
            }
            Ok(UserRamp { name, ramp })
        })
        .collect()
}

fn read_text(path: &Path) -> Result<String, StoreError> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(text)
}

/// 設定のフォルダの `gradients/`。
#[derive(Clone, Debug)]
pub struct RampSetStore {
    dir: PathBuf,
}

impl RampSetStore {
    pub fn new(dir: PathBuf) -> RampSetStore {
        RampSetStore { dir }
    }

    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// 利用者のセットを置く（一時ファイルへ書き、読み戻して確かめてから置換）。1 つも無ければファイルを消す。
    pub fn save(&self, sets: &[UserRamp]) -> Result<(), StoreError> {
        let path = self.path();
        if sets.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            };
        }
        let text = encode(sets);
        let expect: Vec<UserRamp> = sets
            .iter()
            .map(|s| UserRamp {
                name: clean_name(&s.name).unwrap_or_default(),
                ramp: s.ramp.clone(),
            })
            .collect();
        crate::brushes::store::replace_text(&path, &text, MAX_FILE_BYTES, |read| {
            decode(read).is_ok_and(|got| got == expect)
        })
        .map_err(|e| match e {
            crate::brushes::store::StoreError::Io(e) => StoreError::Io(e),
            crate::brushes::store::StoreError::TooLarge => StoreError::TooLarge,
            _ => StoreError::Mismatch,
        })
    }

    /// フォルダのファイルを読む。無ければ空。読めないときは理由（ファイルには触らない）。
    pub fn load(&self) -> Result<Vec<UserRamp>, StoreError> {
        let path = self.path();
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => decode(&read_text(&path)?),
            Ok(_) => Err(StoreError::NotGradients),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(e) => Err(e.into()),
        }
    }
}
