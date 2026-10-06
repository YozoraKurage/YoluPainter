//! ポーズのプリセットの保存（設定のフォルダの `pose_presets/`）。プリセット 1 つが 1 ファイル（`pose-<番号>.ylpose`）。
//!
//! 形式は 1 行目が `yolupainter-pose 1`、あとは `key=value` の行（UTF-8、2 MiB まで）: `name=` 名前（1 つ）、`bone=` 骨 1 つの
//! 休みの形からの差（何行でも）。`bone=` の値は `,` で区切った 平行移動の差 x,y,z（親の空間での足し算）・回転の差 x,y,z,w
//! （単位クォータニオン。骨のローカルの差: 今の回転 = 休みの回転 × 差）・大きさの比 x,y,z（今の大きさ = 休みの大きさ × 比）・
//! 骨の名前の道（根から `/` 区切り。名前の中の `%`・`/`・`,`・改行は `%XX`）。休みの形のままの骨は書かない。項目が 0 のファイルは
//! 休みの形のプリセット。骨はモデルの番号ではなく名前の道で持つので、骨の名前の組が同じモデルへ当てられる（合わない骨は、
//! 当てるときに理由つきで飛ばす。`.ylp` には入れない）。BlendShape の重みは持たない。
//! 知らない項目・名前の重なり・骨の重なり・範囲を外れた値・新しい版のファイルは、そのファイルを読み飛ばして理由を残す
//! （ほかのファイルは読む）。
//! 書き込みは、一時ファイルへ書いて読み戻して確かめてから、プリセットのファイルを 1 回の置換で確定する（途中で落ちても、
//! 前の版か新しい版のどちらか。名前を変える・上書きも同じ）。

use std::collections::HashSet;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use yolu_core::glam::{Quat, Vec3};

use crate::lang::Lang;
use crate::view3d::pose::hide::store::{escape, unescape};

pub const HEADER: &str = "yolupainter-pose 1";
const EXTENSION: &str = "ylpose";
const PREFIX: &str = "pose-";
/// 1 ファイルの大きさの上限。
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// プリセットの数の上限（これより多いファイルは読み飛ばす）。
pub const MAX_PRESETS: usize = 256;
/// 1 つのプリセットの項目の数の上限。
pub const MAX_ENTRIES: usize = 4096;
/// 名前の長さの上限（文字数）。
pub const MAX_NAME_CHARS: usize = 64;
/// 平行移動の差・大きさの比の絶対値の上限（桁の外れた値でモデルを壊さない）。
const MAX_MAGNITUDE: f32 = 1.0e6;
/// 回転の差の長さの許す幅（単位クォータニオン）。
const QUAT_TOLERANCE: f32 = 0.01;

/// 骨 1 つの休みの形からの差。
#[derive(Clone, Debug, PartialEq)]
pub struct PoseEntry {
    /// 骨の名前の道（根から）。
    pub path: Vec<String>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl PoseEntry {
    /// 道を `/` でつないだ表示用の文字。
    pub fn path_text(&self) -> String {
        self.path.join("/")
    }
}

/// 名前つきのポーズ。
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: u32,
    pub name: String,
    pub entries: Vec<PoseEntry>,
}

/// 読み書きの失敗の種類（言語ごとの短い理由は `describe`）。
#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    TooLarge,
    /// 1 行目がこの形式でない。
    NotAPreset,
    /// 今の版より新しい形式。
    NewerVersion(String),
    /// `key=value` でない行（行番号）。
    Syntax(usize),
    UnknownKey(String),
    /// 名前が 2 つ以上。
    DuplicateName,
    /// 値が正しくない（項目の名前）。
    BadValue(String),
    /// 書いたファイルを読み戻したら、書いた中身と違った。
    Mismatch,
    TooMany,
    /// 一覧に無い番号。
    Missing,
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
            StoreError::NotAPreset => lang
                .pick("ポーズのファイルではありません", "Not a pose preset file")
                .into(),
            StoreError::NewerVersion(v) => lang.pick(
                format!("新しい形式です（{v}）"),
                format!("Newer format ({v})"),
            ),
            StoreError::Syntax(line) => lang.pick(
                format!("{line} 行目が読めません"),
                format!("Cannot read line {line}"),
            ),
            StoreError::UnknownKey(key) => lang.pick(
                format!("知らない項目です（{key}）"),
                format!("Unknown item ({key})"),
            ),
            StoreError::DuplicateName => lang
                .pick("名前が重なっています", "The name appears twice")
                .into(),
            StoreError::BadValue(key) => lang.pick(
                format!("値が正しくありません（{key}）"),
                format!("Invalid value ({key})"),
            ),
            StoreError::Mismatch => lang
                .pick(
                    "書いた中身を読み戻せません",
                    "Written file does not read back",
                )
                .into(),
            StoreError::TooMany => lang.pick("ポーズが多すぎます", "Too many poses").into(),
            StoreError::Missing => lang
                .pick("一覧にないポーズです", "The pose is not in the list")
                .into(),
        }
    }
}

/// 読めなかったファイルと理由。
#[derive(Debug)]
pub struct Problem {
    pub file: String,
    pub error: StoreError,
}

impl Problem {
    pub fn describe(&self, lang: Lang) -> String {
        format!("{}: {}", self.file, self.error.describe(lang))
    }
}

/// ポーズのプリセットの一覧（設定のフォルダが分からないときは、保存せずにこの起動の間だけ持つ）。
#[derive(Debug, Default)]
pub struct Presets {
    dir: Option<PathBuf>,
    items: Vec<Preset>,
    /// 読み込みのときに読めなかったファイル。
    pub problems: Vec<Problem>,
    next_id: u32,
}

impl Presets {
    /// 設定のフォルダのプリセットを読む（起動のとき 1 回。読めないファイルは読み飛ばして理由を残す）。
    pub fn attach(&mut self, dir: PathBuf) {
        self.items.clear();
        self.problems.clear();
        let mut files: Vec<(u32, PathBuf)> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&dir) {
            for entry in read.flatten() {
                let path = entry.path();
                if let Some(id) = file_id(&path) {
                    files.push((id, path));
                }
            }
        }
        files.sort_by_key(|(id, _)| *id);
        let mut max_id = 0;
        for (id, path) in files {
            max_id = max_id.max(id);
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if self.items.len() >= MAX_PRESETS {
                self.problems.push(Problem {
                    file,
                    error: StoreError::TooMany,
                });
                continue;
            }
            match read_file(&path) {
                Ok((name, entries)) => {
                    if self.items.iter().any(|p| p.name == name) {
                        self.problems.push(Problem {
                            file,
                            error: StoreError::DuplicateName,
                        });
                    } else {
                        self.items.push(Preset { id, name, entries });
                    }
                }
                Err(error) => self.problems.push(Problem { file, error }),
            }
        }
        self.next_id = max_id + 1;
        self.dir = Some(dir);
    }

    pub fn items(&self) -> &[Preset] {
        &self.items
    }

    pub fn get(&self, id: u32) -> Option<&Preset> {
        self.items.iter().find(|p| p.id == id)
    }

    /// 保存先のフォルダ（無ければ、この起動の間だけ）。
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// 名前が重ならないように、後ろに番号を付ける（既にある名前でなければそのまま）。番号を付けても名前の長さの上限（読み戻しの上限と
    /// 同じ）を超えないように、番号の分だけ元の名前を切る。`except` の番号のプリセットの名前は、重ねてよい（自分自身）。
    fn unique_name_except(&self, base: &str, except: Option<u32>) -> String {
        let taken = |name: &str| {
            self.items
                .iter()
                .any(|p| Some(p.id) != except && p.name == name)
        };
        let base: String = base.trim().chars().take(MAX_NAME_CHARS).collect();
        if !taken(&base) {
            return base;
        }
        (2..)
            .map(|n| {
                let suffix = format!(" {n}");
                let keep = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
                let stem: String = base.chars().take(keep).collect();
                format!("{}{suffix}", stem.trim_end())
            })
            .find(|name| !taken(name))
            .unwrap_or(base)
    }

    pub fn unique_name(&self, base: &str) -> String {
        self.unique_name_except(base, None)
    }

    /// 新しいプリセットを保存する（名前が重なるときは番号を付ける。項目が 0 でもよい: 休みの形）。保存できたらその番号。
    pub fn add(&mut self, name: &str, entries: Vec<PoseEntry>) -> Result<u32, StoreError> {
        if name.trim().is_empty() {
            return Err(StoreError::BadValue("name".into()));
        }
        if entries.len() > MAX_ENTRIES || self.items.len() >= MAX_PRESETS {
            return Err(StoreError::TooMany);
        }
        let preset = Preset {
            id: self.next_id.max(1),
            name: self.unique_name(name),
            entries,
        };
        if let Some(dir) = &self.dir {
            write_file(dir, &preset)?;
        }
        self.next_id = preset.id + 1;
        let id = preset.id;
        self.items.push(preset);
        Ok(id)
    }

    /// 項目を入れ替える（名前と番号はそのまま。書けなければ、ファイルも一覧も前のまま）。
    pub fn replace(&mut self, id: u32, entries: Vec<PoseEntry>) -> Result<(), StoreError> {
        let at = self
            .items
            .iter()
            .position(|p| p.id == id)
            .ok_or(StoreError::Missing)?;
        if entries.len() > MAX_ENTRIES {
            return Err(StoreError::TooMany);
        }
        let preset = Preset {
            id,
            name: self.items[at].name.clone(),
            entries,
        };
        if let Some(dir) = &self.dir {
            write_file(dir, &preset)?;
        }
        self.items[at] = preset;
        Ok(())
    }

    /// 名前を変える（ほかのプリセットと重なるときは番号を付ける。書けなければ、ファイルも一覧も前のまま）。変えたあとの名前。
    pub fn rename(&mut self, id: u32, name: &str) -> Result<String, StoreError> {
        if name.trim().is_empty() {
            return Err(StoreError::BadValue("name".into()));
        }
        let at = self
            .items
            .iter()
            .position(|p| p.id == id)
            .ok_or(StoreError::Missing)?;
        let unique = self.unique_name_except(name, Some(id));
        if unique == self.items[at].name {
            return Ok(unique);
        }
        let preset = Preset {
            id,
            name: unique.clone(),
            entries: self.items[at].entries.clone(),
        };
        if let Some(dir) = &self.dir {
            write_file(dir, &preset)?;
        }
        self.items[at] = preset;
        Ok(unique)
    }

    /// プリセットを消す（ファイルも。利用者が消すと選んだものだけ）。
    pub fn remove(&mut self, id: u32) -> Result<(), StoreError> {
        let Some(at) = self.items.iter().position(|p| p.id == id) else {
            return Ok(());
        };
        if let Some(dir) = &self.dir {
            match std::fs::remove_file(path_of(dir, id)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
        }
        self.items.remove(at);
        Ok(())
    }
}

fn file_name(id: u32) -> String {
    format!("{PREFIX}{id}.{EXTENSION}")
}

fn path_of(dir: &Path, id: u32) -> PathBuf {
    dir.join(file_name(id))
}

/// `pose-<番号>.ylpose` の番号。
fn file_id(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(&format!(".{EXTENSION}"))?;
    stem.strip_prefix(PREFIX)?.parse().ok()
}

// ───────── 形式 ─────────

/// ファイルの中身。
pub fn render(name: &str, entries: &[PoseEntry]) -> String {
    let mut text = format!("{HEADER}\nname={}\n", escape(name));
    for e in entries {
        let path: Vec<String> = e.path.iter().map(|c| escape(c)).collect();
        let (t, q, s) = (e.translation, e.rotation, e.scale);
        text += &format!(
            "bone={},{},{},{},{},{},{},{},{},{},{}\n",
            t.x,
            t.y,
            t.z,
            q.x,
            q.y,
            q.z,
            q.w,
            s.x,
            s.y,
            s.z,
            path.join("/")
        );
    }
    text
}

/// `bone=` の値 1 行。
fn parse_entry(value: &str) -> Option<PoseEntry> {
    let mut parts = value.splitn(11, ',');
    let mut number = || -> Option<f32> {
        let v: f32 = parts.next()?.parse().ok()?;
        v.is_finite().then_some(v)
    };
    let t = Vec3::new(number()?, number()?, number()?);
    let q = Quat::from_xyzw(number()?, number()?, number()?, number()?);
    let s = Vec3::new(number()?, number()?, number()?);
    let path_text = parts.next()?;
    if t.abs().max_element() > MAX_MAGNITUDE || s.abs().max_element() > MAX_MAGNITUDE {
        return None;
    }
    if (q.length() - 1.0).abs() > QUAT_TOLERANCE {
        return None;
    }
    let path: Vec<String> = path_text.split('/').map(unescape).collect::<Option<_>>()?;
    if path.iter().any(|c| c.is_empty()) {
        return None;
    }
    Some(PoseEntry {
        path,
        translation: t,
        rotation: q,
        scale: s,
    })
}

/// ファイルの中身を読む（名前と項目）。
pub fn parse(text: &str) -> Result<(String, Vec<PoseEntry>), StoreError> {
    let mut lines = text.lines();
    match lines.next() {
        Some(HEADER) => {}
        Some(first) if first.starts_with("yolupainter-pose ") => {
            return Err(StoreError::NewerVersion(first.to_owned()));
        }
        _ => return Err(StoreError::NotAPreset),
    }
    let mut name: Option<String> = None;
    let mut entries: Vec<PoseEntry> = Vec::new();
    let mut paths: HashSet<Vec<String>> = HashSet::new();
    for (i, line) in lines.enumerate() {
        let number = i + 2;
        if line.is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(StoreError::Syntax(number))?;
        match key {
            "name" => {
                if name.is_some() {
                    return Err(StoreError::DuplicateName);
                }
                let value = unescape(value).ok_or_else(|| StoreError::BadValue("name".into()))?;
                if value.trim().is_empty() || value.chars().count() > MAX_NAME_CHARS {
                    return Err(StoreError::BadValue("name".into()));
                }
                name = Some(value);
            }
            "bone" => {
                if entries.len() >= MAX_ENTRIES {
                    return Err(StoreError::TooMany);
                }
                let entry =
                    parse_entry(value).ok_or_else(|| StoreError::BadValue("bone".into()))?;
                if !paths.insert(entry.path.clone()) {
                    // 同じ骨の項目が 2 つ（どちらを当てるか決まらない）
                    return Err(StoreError::BadValue("bone".into()));
                }
                entries.push(entry);
            }
            other => return Err(StoreError::UnknownKey(other.to_owned())),
        }
    }
    let name = name.ok_or_else(|| StoreError::BadValue("name".into()))?;
    Ok((name, entries))
}

fn read_file(path: &Path) -> Result<(String, Vec<PoseEntry>), StoreError> {
    let meta = std::fs::metadata(path)?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    let bytes = std::fs::read(path)?;
    let text = String::from_utf8(bytes).map_err(|_| StoreError::NotAPreset)?;
    parse(&text)
}

/// 一時ファイルへ書き、読み戻して確かめてから、プリセットのファイルへ 1 回の置換で確定する（新しい番号でも、名前を変える・
/// 上書きの置き換えでも同じ）。
fn write_file(dir: &Path, preset: &Preset) -> Result<(), StoreError> {
    std::fs::create_dir_all(dir)?;
    let text = render(&preset.name, &preset.entries);
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(StoreError::TooLarge);
    }
    let verify = |read: &[u8]| {
        std::str::from_utf8(read)
            .ok()
            .and_then(|text| parse(text).ok())
            .is_some_and(|(name, entries)| name == preset.name && entries == preset.entries)
    };
    let opts = yolu_io::atomic::ReplaceOptions {
        limit: Some(MAX_FILE_BYTES),
        verify: Some(&verify),
        create_dirs: false,
    };
    yolu_io::atomic::replace_with(&path_of(dir, preset.id), &opts, |f| {
        f.write_all(text.as_bytes())
    })
    .map_err(|e| match yolu_io::atomic::rejected(&e) {
        Some(yolu_io::atomic::Rejected::TooLarge) => StoreError::TooLarge,
        Some(yolu_io::atomic::Rejected::Mismatch) => StoreError::Mismatch,
        None => StoreError::Io(e),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("yolu-pose-store-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(path: &[&str], x: f32) -> PoseEntry {
        PoseEntry {
            path: path.iter().map(|s| s.to_string()).collect(),
            translation: Vec3::new(x, 0.25, -0.5),
            rotation: Quat::from_rotation_z(0.7 * x + 0.1),
            scale: Vec3::new(1.0, 1.5, 0.75),
        }
    }

    #[test]
    fn a_preset_round_trips_through_the_file_text_including_awkward_names() {
        let entries = vec![
            entry(&["腰", "背骨", "頭"], 0.3),
            entry(&["Armature/Root", "a,b", "100%\n改行"], -0.2),
        ];
        let text = render("ポーズ・耳 / 100%", &entries);
        assert!(text.starts_with("yolupainter-pose 1\n"));
        assert_eq!(text.lines().count(), 4, "改行を含む名前も 1 行: {text}");
        let (name, back) = parse(&text).unwrap();
        assert_eq!(name, "ポーズ・耳 / 100%");
        assert_eq!(back, entries, "f32 は書いたとおりに読み戻る");
        // 項目が 0 のプリセット（休みの形）も書いて読める
        let (name, back) = parse(&render("休み", &[])).unwrap();
        assert_eq!((name.as_str(), back.len()), ("休み", 0));
    }

    #[test]
    fn broken_files_are_refused_with_a_reason() {
        let good = render("名前", &[entry(&["根"], 0.3)]);
        assert!(matches!(parse(""), Err(StoreError::NotAPreset)));
        assert!(matches!(
            parse("yolupainter-hide 1\n"),
            Err(StoreError::NotAPreset)
        ));
        assert!(matches!(
            parse("yolupainter-pose 2\nname=x\n"),
            Err(StoreError::NewerVersion(_))
        ));
        assert!(matches!(
            parse(&format!("{good}name=二つ目\n")),
            Err(StoreError::DuplicateName)
        ));
        assert!(matches!(
            parse(&format!("{good}color=red\n")),
            Err(StoreError::UnknownKey(_))
        ));
        assert!(matches!(
            parse(&format!("{good}ただの行\n")),
            Err(StoreError::Syntax(4))
        ));
        let ok = "0,0,0,0,0,0,1,1,1,1";
        for bad in [
            // 数が足りない・多すぎる・数でない・有限でない
            "bone=0,0,0,0,0,0,1,1,1,根".to_string(),
            "bone=0,0,0,0,0,0,1,1,1,x,根".to_string(),
            "bone=NaN,0,0,0,0,0,1,1,1,1,根".to_string(),
            "bone=inf,0,0,0,0,0,1,1,1,1,根".to_string(),
            // 単位でない回転
            "bone=0,0,0,0,0,0,0,1,1,1,根".to_string(),
            "bone=0,0,0,1,1,1,1,1,1,1,根".to_string(),
            // 桁の外れた値
            "bone=2000000,0,0,0,0,0,1,1,1,1,根".to_string(),
            "bone=0,0,0,0,0,0,1,1,1,-2000000,根".to_string(),
            // 道が空・空の名前・正しくない %
            format!("bone={ok},"),
            format!("bone={ok},根//子"),
            format!("bone={ok},%ZZ"),
        ] {
            assert!(
                matches!(
                    parse(&format!("yolupainter-pose 1\nname=名前\n{bad}\n")),
                    Err(StoreError::BadValue(_))
                ),
                "{bad}"
            );
        }
        // 同じ骨の項目が 2 つ
        assert!(matches!(
            parse(&format!("{good}bone={ok},根\n")),
            Err(StoreError::BadValue(_))
        ));
        assert!(matches!(
            parse("yolupainter-pose 1\nbone=0,0,0,0,0,0,1,1,1,1,根\n"),
            Err(StoreError::BadValue(_))
        ));
        let long = "あ".repeat(MAX_NAME_CHARS + 1);
        assert!(matches!(
            parse(&format!("yolupainter-pose 1\nname={long}\n")),
            Err(StoreError::BadValue(_))
        ));
        for e in [
            StoreError::Missing,
            StoreError::TooMany,
            StoreError::Mismatch,
            StoreError::TooLarge,
        ] {
            assert!(!e.describe(Lang::Ja).is_empty() && e.describe(Lang::En).is_ascii());
        }
    }

    #[test]
    fn saving_writes_a_file_and_loading_reads_it_back_in_order() {
        let dir = dir("save");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        assert!(presets.items().is_empty());
        let a = presets
            .add("構え", vec![entry(&["腰", "頭"], 0.3)])
            .unwrap();
        let b = presets
            .add("座り", vec![entry(&["腰", "胸"], 0.5)])
            .unwrap();
        assert!(b > a);
        assert!(dir.join(format!("pose-{a}.ylpose")).exists());
        // 一時ファイルは残さない
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.ends_with(".pending") || name.ends_with(".pending~")
            })
            .collect();
        assert!(leftovers.is_empty());
        let mut again = Presets::default();
        again.attach(dir.clone());
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        assert_eq!(again.items(), presets.items());
        // 番号は続きから（消した最大の番号を使い回さない）
        again.remove(b).unwrap();
        assert!(!dir.join(format!("pose-{b}.ylpose")).exists());
        let c = again.add("立ち", vec![entry(&["腰"], 0.1)]).unwrap();
        assert!(c > b, "消した {b} を使い回さない（{c}）");
        let mut reloaded = Presets::default();
        reloaded.attach(dir.clone());
        assert_eq!(reloaded.items().last().unwrap().id, c);
        // 消すのは選んだ 1 つだけ
        assert!(again.get(a).is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_name_that_exists_gets_a_number_instead_of_replacing_the_old_preset() {
        let dir = dir("names");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        let e = || vec![entry(&["根"], 0.3)];
        presets.add("ポーズ", e()).unwrap();
        presets.add("ポーズ", e()).unwrap();
        presets.add(" ポーズ ", e()).unwrap();
        let names: Vec<&str> = presets.items().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["ポーズ", "ポーズ 2", "ポーズ 3"]);
        assert!(matches!(
            presets.add("  ", e()),
            Err(StoreError::BadValue(_))
        ));
        // 休みの形（項目が 0）のプリセットも足せる
        assert!(presets.add("休み", Vec::new()).is_ok());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_numbered_name_stays_within_the_name_limit_and_reads_back() {
        let dir = dir("longname");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        let e = || vec![entry(&["根"], 0.3)];
        let long = "あ".repeat(MAX_NAME_CHARS);
        let a = presets.add(&long, e()).unwrap();
        let b = presets.add(&long, e()).unwrap();
        for p in presets.items() {
            assert!(p.name.chars().count() <= MAX_NAME_CHARS, "{}", p.name);
        }
        assert_eq!(presets.get(a).unwrap().name, long);
        assert!(presets.get(b).unwrap().name.ends_with("あ 2"));
        let mut again = Presets::default();
        again.attach(dir.clone());
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        assert_eq!(again.items(), presets.items());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_and_replace_rewrite_one_file_and_keep_the_id() {
        let dir = dir("rename");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        let a = presets.add("構え", vec![entry(&["腰"], 0.3)]).unwrap();
        let b = presets.add("座り", vec![entry(&["腰"], 0.5)]).unwrap();
        // 名前を変える: 番号と項目はそのまま、ファイルは同じ番号
        assert_eq!(presets.rename(a, "  走り ").unwrap(), "走り");
        assert_eq!(presets.get(a).unwrap().entries, vec![entry(&["腰"], 0.3)]);
        // 同じ名前なら何も変えない（番号を付けない）
        assert_eq!(presets.rename(a, "走り").unwrap(), "走り");
        // 別のプリセットの名前と重なれば番号を付ける
        assert_eq!(presets.rename(a, "座り").unwrap(), "座り 2");
        assert!(matches!(
            presets.rename(a, " "),
            Err(StoreError::BadValue(_))
        ));
        assert!(matches!(presets.rename(999, "x"), Err(StoreError::Missing)));
        // 上書き: 名前はそのまま、項目だけ入れ替わる
        presets
            .replace(b, vec![entry(&["腰", "頭"], -0.4)])
            .unwrap();
        assert_eq!(presets.get(b).unwrap().name, "座り");
        assert!(matches!(
            presets.replace(999, Vec::new()),
            Err(StoreError::Missing)
        ));
        let many: Vec<PoseEntry> = (0..=MAX_ENTRIES).map(|_| entry(&["根"], 0.1)).collect();
        assert!(matches!(presets.replace(b, many), Err(StoreError::TooMany)));
        // 読み直しても同じ・ファイルは 2 つのまま・一時ファイルは残らない
        let mut again = Presets::default();
        again.attach(dir.clone());
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        assert_eq!(again.items(), presets.items());
        let files: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(files.len(), 2, "{files:?}");
        assert!(files.iter().all(|f| f.ends_with(".ylpose")), "{files:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn too_many_presets_or_entries_are_refused_on_load_parse_and_add() {
        let dir = dir("toomany");
        std::fs::create_dir_all(&dir).unwrap();
        for id in 1..=(MAX_PRESETS + 1) {
            std::fs::write(
                dir.join(format!("pose-{id}.ylpose")),
                render(&format!("p{id}"), &[entry(&["根"], 0.3)]),
            )
            .unwrap();
        }
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        assert_eq!(presets.items().len(), MAX_PRESETS);
        assert_eq!(presets.problems.len(), 1);
        assert!(matches!(presets.problems[0].error, StoreError::TooMany));
        assert_eq!(
            presets.problems[0].file,
            format!("pose-{}.ylpose", MAX_PRESETS + 1)
        );
        assert!(
            dir.join(format!("pose-{}.ylpose", MAX_PRESETS + 1))
                .exists(),
            "読み飛ばしたファイルには触らない"
        );
        let err = presets.add("あふれ", vec![entry(&["根"], 0.3)]);
        assert!(matches!(err, Err(StoreError::TooMany)));
        assert_eq!(presets.items().len(), MAX_PRESETS);
        std::fs::remove_dir_all(dir).unwrap();

        // 項目が多すぎる add・ちょうどの add
        let mut few = Presets::default();
        let distinct = |n: usize| -> Vec<PoseEntry> {
            (0..n)
                .map(|i| PoseEntry {
                    path: vec![format!("b{i}")],
                    ..entry(&["x"], 0.1)
                })
                .collect()
        };
        assert!(matches!(
            few.add("多い", distinct(MAX_ENTRIES + 1)),
            Err(StoreError::TooMany)
        ));
        assert!(few.items().is_empty());
        assert!(few.add("ちょうど", distinct(MAX_ENTRIES)).is_ok());
        // parse: 項目の数の上限
        let text = |n: usize| {
            let mut t = format!("{HEADER}\nname=x\n");
            for i in 0..n {
                t += &format!("bone=0,0,0,0,0,0,1,1,1,1,b{i}\n");
            }
            t
        };
        assert_eq!(parse(&text(MAX_ENTRIES)).unwrap().1.len(), MAX_ENTRIES);
        assert!(matches!(
            parse(&text(MAX_ENTRIES + 1)),
            Err(StoreError::TooMany)
        ));
    }

    #[test]
    fn unreadable_files_are_skipped_with_a_reason_and_the_rest_load() {
        let dir = dir("problems");
        std::fs::create_dir_all(&dir).unwrap();
        let good = render("良い", &[entry(&["根"], 0.3)]);
        std::fs::write(dir.join("pose-1.ylpose"), &good).unwrap();
        std::fs::write(dir.join("pose-2.ylpose"), "yolupainter-pose 9\n").unwrap();
        std::fs::write(dir.join("pose-3.ylpose"), "壊れた").unwrap();
        std::fs::write(
            dir.join("pose-4.ylpose"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        std::fs::write(dir.join("memo.txt"), "関係ないファイル").unwrap();
        std::fs::write(dir.join("pose-5.ylpose"), &good).unwrap();
        std::fs::write(dir.join("hide-6.ylhide"), "隠し方のファイルは読まない").unwrap();
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        assert_eq!(presets.items().len(), 1);
        assert_eq!(presets.items()[0].name, "良い");
        let reasons: Vec<(String, &'static str)> = presets
            .problems
            .iter()
            .map(|p| {
                (
                    p.file.clone(),
                    match p.error {
                        StoreError::NewerVersion(_) => "newer",
                        StoreError::NotAPreset => "not",
                        StoreError::TooLarge => "large",
                        StoreError::DuplicateName => "dup",
                        _ => "other",
                    },
                )
            })
            .collect();
        assert_eq!(
            reasons,
            [
                ("pose-2.ylpose".to_string(), "newer"),
                ("pose-3.ylpose".to_string(), "not"),
                ("pose-4.ylpose".to_string(), "large"),
                ("pose-5.ylpose".to_string(), "dup"),
            ]
        );
        // 読めなかったファイルには触らない
        assert!(dir.join("pose-2.ylpose").exists());
        // 番号は最大の続き（読めなかったファイルの番号も使わない）
        let id = presets.add("新しい", vec![entry(&["根"], 0.3)]).unwrap();
        assert_eq!(id, 6);
        for p in &presets.problems {
            assert!(!p.describe(Lang::En).is_empty() && !p.describe(Lang::Ja).is_empty());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn without_a_folder_presets_live_in_memory() {
        let mut presets = Presets::default();
        assert!(presets.dir().is_none());
        let id = presets
            .add("メモリだけ", vec![entry(&["根"], 0.3)])
            .unwrap();
        assert_eq!(presets.get(id).unwrap().name, "メモリだけ");
        assert_eq!(presets.rename(id, "改名").unwrap(), "改名");
        presets.replace(id, Vec::new()).unwrap();
        assert!(presets.get(id).unwrap().entries.is_empty());
        presets.remove(id).unwrap();
        assert!(presets.items().is_empty());
        presets.remove(999).unwrap();
    }

    #[test]
    fn a_write_that_cannot_happen_leaves_the_list_unchanged() {
        let dir = dir("blocked");
        std::fs::create_dir_all(&dir).unwrap();
        // フォルダの場所にファイルがあって作れない
        let blocked = dir.join("pose_presets");
        std::fs::write(&blocked, "ファイル").unwrap();
        let mut presets = Presets::default();
        presets.attach(blocked.clone());
        let err = presets.add("書けない", vec![entry(&["根"], 0.3)]);
        assert!(matches!(err, Err(StoreError::Io(_))));
        assert!(presets.items().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// 名前を変える・上書きが書けないときは、前のファイルも一覧もそのまま（置換の前に失敗する）。
    #[test]
    fn a_rename_or_replace_that_cannot_be_written_keeps_the_old_file_and_list() {
        let dir = dir("keep");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        let id = presets.add("元", vec![entry(&["腰"], 0.3)]).unwrap();
        let before = std::fs::read(dir.join(format!("pose-{id}.ylpose"))).unwrap();
        // 置き換える前に失敗させる
        yolu_io::atomic::failing(|| {
            assert!(presets.rename(id, "先").is_err());
            assert!(presets.replace(id, Vec::new()).is_err());
        });
        assert_eq!(presets.get(id).unwrap().name, "元");
        assert_eq!(presets.get(id).unwrap().entries.len(), 1);
        assert_eq!(
            std::fs::read(dir.join(format!("pose-{id}.ylpose"))).unwrap(),
            before,
            "前のファイルのまま"
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
