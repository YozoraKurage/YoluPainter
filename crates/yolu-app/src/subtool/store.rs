//! 利用者のサブツールのプリセットの保存（設定のフォルダの `subtools/`）。ツールごとに 1 ファイル（`<ツール>.ylsubtool`。ツールは `fill`・
//! `polygon-fill`・`gradient`・`shape`・`ruler`・`eyedropper`・`move`・`liquify`）。ブラシと消しゴムは `.ylbrush`（`brushes::store`）で、ここには入れない。
//!
//! 形式は 1 行目が `yolupainter-subtools 1`、あとは `key=value` の行（UTF-8、512 KiB まで、空行は読み飛ばす）。
//! ```text
//! yolupainter-subtools 1
//! tool=fill
//! preset.1.name=自分のバケツ
//! preset.1.by_color=true
//! preset.1.tolerance=48
//! ```
//! `tool` は 2 行目で、ファイルの名前のツールと同じこと。`preset.<番号>.name` は名前（40 文字まで）、ほかはツールの欄（`fields`）の名前。
//! 数は Rust の表記のまま書き（読み戻しても同じ値）、選びは名前（`kind=triangle` など）。書かれていない欄は既定で読む（欄が増えても
//! 古いファイルを読める）。番号は 1 から、プリセットは番号の順。知らない項目・重なった項目・範囲を外れた値・名前のないプリセット・
//! 別のツールのファイル・多すぎるプリセットは、そのファイルを読み飛ばして理由を残す（ほかのツールのファイルは読む）。版が新しいファイルは
//! 触らずに読み飛ばす。書き込みは、一時ファイルへ書いて読み戻して確かめてから、最後の 1 回の置換で確定する（途中で落ちても前の版が残る）。
//! 一覧の並び（組み込みが先、利用者のものが番号の順）と「変えたままの設定」は保存しない。

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use super::fields::{fields, TOOLS};
use super::{Field, Kind, UserPreset, Value, Values};
use crate::brushes::clean_name;
use crate::lang::Lang;
use crate::state::Tool;

pub const HEADER: &str = "yolupainter-subtools 1";
const EXTENSION: &str = "ylsubtool";
/// 1 ファイルの大きさの上限。プリセットの数の上限（`MAX_USER_PRESETS`）まで、どのツールのどんな値でも収まる大きさ（試験が、いちばん長い書き方で確かめる）。
/// 数の上限に先に当たるので、足したプリセットだけがメモリに残って保存できない、ということが起きない。
pub const MAX_FILE_BYTES: u64 = 512 * 1024;
/// 1 つのツールの利用者のプリセットの数の上限。
pub const MAX_USER_PRESETS: usize = 256;

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    TooLarge,
    /// 1 行目がこの形式でない。
    NotSubTools,
    /// 今の版より新しい形式。
    NewerVersion(String),
    /// `key=value` でない行（行番号）。
    Syntax(usize),
    UnknownKey(String),
    DuplicateKey(String),
    BadValue(String),
    /// 別のツールのファイル。
    WrongTool,
    TooMany,
    /// 書いたファイルを読み戻したら、書いた設定と違った。
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
            StoreError::NotSubTools => lang
                .pick("サブツールのファイルではありません", "Not a sub tool file")
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
            StoreError::WrongTool => lang
                .pick(
                    "別のツールのファイルです",
                    "The file belongs to another tool",
                )
                .into(),
            StoreError::TooMany => lang
                .pick("プリセットが多すぎます", "Too many presets")
                .into(),
            StoreError::Mismatch => lang
                .pick(
                    "書いた内容を読み戻せませんでした",
                    "The written file did not read back the same",
                )
                .into(),
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
    pub presets: Vec<(Tool, Vec<UserPreset>)>,
    pub problems: Vec<Problem>,
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Choice(s) => (*s).to_owned(),
    }
}

fn parse_value(field: &Field, text: &str, key: &str) -> Result<Value, StoreError> {
    let bad = || StoreError::BadValue(key.to_owned());
    Ok(match field.kind {
        Kind::Bool => match text {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(bad()),
        },
        Kind::Int(min, max) => {
            let n: i32 = text.parse().map_err(|_| bad())?;
            if !(min..=max).contains(&n) {
                return Err(bad());
            }
            Value::Int(n)
        }
        Kind::Float(min, max) => {
            let f: f32 = text.parse().map_err(|_| bad())?;
            if !f.is_finite() || !(min..=max).contains(&f) {
                return Err(bad());
            }
            Value::Float(f)
        }
        Kind::Choice(options) => match options.iter().find(|o| **o == text) {
            Some(o) => Value::Choice(o),
            None => return Err(bad()),
        },
    })
}

/// 利用者のプリセットを書く文にする。
pub fn encode(tool: Tool, presets: &[UserPreset]) -> String {
    let mut text = format!("{HEADER}\ntool={}\n", tool.id());
    for preset in presets {
        text.push_str(&format!(
            "preset.{}.name={}\n",
            preset.id,
            clean_name(&preset.name).unwrap_or_default()
        ));
        for (field, value) in fields(tool).iter().zip(&preset.values) {
            text.push_str(&format!(
                "preset.{}.{}={}\n",
                preset.id,
                field.key,
                text_of(value)
            ));
        }
    }
    text
}

/// 文を読む。ファイルのツールが `tool` と違えば断る。
pub fn decode(tool: Tool, text: &str) -> Result<Vec<UserPreset>, StoreError> {
    let mut lines = text.lines().enumerate();
    let first = lines.next().map(|(_, l)| l.trim_end()).unwrap_or("");
    if first != HEADER {
        return match first.strip_prefix("yolupainter-subtools ") {
            Some(version) => Err(StoreError::NewerVersion(version.trim().to_owned())),
            None => Err(StoreError::NotSubTools),
        };
    }
    let table = fields(tool);
    let mut seen_tool = false;
    // 番号 → (名前, 欄ごとの文)
    let mut found: BTreeMap<u32, (Option<String>, Vec<Option<Value>>)> = BTreeMap::new();
    for (index, line) in lines {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let (key, value) = line.split_once('=').ok_or(StoreError::Syntax(index + 1))?;
        if key == "tool" {
            if seen_tool {
                return Err(StoreError::DuplicateKey(key.into()));
            }
            if value != tool.id() {
                return Err(StoreError::WrongTool);
            }
            seen_tool = true;
            continue;
        }
        if !seen_tool {
            // ツールの行が先でないファイルは、どのツールのものか決められない
            return Err(StoreError::Syntax(index + 1));
        }
        let rest = key
            .strip_prefix("preset.")
            .ok_or_else(|| StoreError::UnknownKey(key.into()))?;
        let (number, name) = rest
            .split_once('.')
            .ok_or_else(|| StoreError::UnknownKey(key.into()))?;
        let id: u32 = number
            .parse()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| StoreError::UnknownKey(key.into()))?;
        if !found.contains_key(&id) && found.len() >= MAX_USER_PRESETS {
            return Err(StoreError::TooMany);
        }
        let entry = found
            .entry(id)
            .or_insert_with(|| (None, vec![None; table.len()]));
        if name == "name" {
            if entry.0.is_some() {
                return Err(StoreError::DuplicateKey(key.into()));
            }
            entry.0 = Some(clean_name(value).ok_or_else(|| StoreError::BadValue(key.into()))?);
            continue;
        }
        let at = table
            .iter()
            .position(|f| f.key == name)
            .ok_or_else(|| StoreError::UnknownKey(key.into()))?;
        if entry.1[at].is_some() {
            return Err(StoreError::DuplicateKey(key.into()));
        }
        entry.1[at] = Some(parse_value(&table[at], value, key)?);
    }
    if !seen_tool {
        return Err(StoreError::WrongTool);
    }
    found
        .into_iter()
        .map(|(id, (name, values))| {
            let name = name.ok_or_else(|| StoreError::BadValue(format!("preset.{id}.name")))?;
            let values: Values = values
                .into_iter()
                .zip(table)
                .map(|(v, f)| v.unwrap_or_else(|| f.default.clone()))
                .collect();
            Ok(UserPreset { id, name, values })
        })
        .collect()
}

fn file_name(tool: Tool) -> String {
    format!("{}.{EXTENSION}", tool.id())
}

fn read_text(path: &Path) -> Result<String, StoreError> {
    crate::userfiles::read_text(path, MAX_FILE_BYTES).map_err(|e| match e {
        crate::userfiles::FileError::TooLarge => StoreError::TooLarge,
        crate::userfiles::FileError::Io(e) => StoreError::Io(e),
        // read_text は文でない中身を読み込みの失敗（Io）で返し、確かめの失敗は返さない
        crate::userfiles::FileError::NotText | crate::userfiles::FileError::Mismatch => {
            StoreError::Io(std::io::ErrorKind::InvalidData.into())
        }
    })
}

/// 設定のフォルダの `subtools/`。
#[derive(Clone, Debug)]
pub struct SubToolStore {
    dir: PathBuf,
}

impl SubToolStore {
    pub fn new(dir: PathBuf) -> SubToolStore {
        SubToolStore { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path_of(&self, tool: Tool) -> PathBuf {
        self.dir.join(file_name(tool))
    }

    /// ツールの利用者のプリセットを置く（一時ファイルへ書き、読み戻して確かめてから置換）。プリセットが 1 つも無ければファイルを消す。
    pub fn save(&self, tool: Tool, presets: &[UserPreset]) -> Result<(), StoreError> {
        let path = self.path_of(tool);
        if presets.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.into()),
            };
        }
        let text = encode(tool, presets);
        let expect: Vec<UserPreset> = presets
            .iter()
            .map(|p| UserPreset {
                id: p.id,
                name: clean_name(&p.name).unwrap_or_default(),
                values: p.values.clone(),
            })
            .collect();
        crate::brushes::store::replace_text(&path, &text, MAX_FILE_BYTES, |read| {
            decode(tool, read).is_ok_and(|got| got == expect)
        })
        .map_err(|e| match e {
            crate::brushes::store::StoreError::Io(e) => StoreError::Io(e),
            crate::brushes::store::StoreError::TooLarge => StoreError::TooLarge,
            _ => StoreError::Mismatch,
        })
    }
}

/// フォルダのツールごとのファイルを全部読む。読めないファイルは読み飛ばして `problems` に残す。フォルダが無ければ空。
pub fn load_all(dir: &Path) -> LoadReport {
    let mut report = LoadReport::default();
    for tool in TOOLS {
        let name = file_name(tool);
        let path = dir.join(&name);
        let loaded = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() => read_text(&path).and_then(|text| decode(tool, &text)),
            Ok(_) => Err(StoreError::NotSubTools),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(e) => Err(e.into()),
        };
        match loaded {
            Ok(presets) => report.presets.push((tool, presets)),
            Err(reason) => report.problems.push(Problem { file: name, reason }),
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subtool::fields::builtins;

    fn defaults(tool: Tool) -> Values {
        fields(tool).iter().map(|f| f.default.clone()).collect()
    }

    fn preset(tool: Tool, id: u32, name: &str) -> UserPreset {
        UserPreset {
            id,
            name: name.into(),
            values: defaults(tool),
        }
    }

    #[test]
    fn every_tool_round_trips_with_every_field_changed() {
        for tool in TOOLS {
            let mut values = defaults(tool);
            for (value, field) in values.iter_mut().zip(fields(tool)) {
                *value = match field.kind {
                    Kind::Bool => Value::Bool(!matches!(field.default, Value::Bool(true))),
                    Kind::Int(min, max) => Value::Int(if field.default == Value::Int(max) {
                        min
                    } else {
                        max
                    }),
                    Kind::Float(min, max) => Value::Float(if field.default == Value::Float(max) {
                        min
                    } else {
                        max - 0.25
                    }),
                    Kind::Choice(options) => Value::Choice(options[options.len() - 1]),
                };
            }
            let presets = vec![
                UserPreset {
                    id: 2,
                    name: "自分の設定 = 2".into(),
                    values,
                },
                preset(tool, 7, "plain"),
            ];
            let text = encode(tool, &presets);
            assert_eq!(decode(tool, &text).unwrap(), presets, "{tool:?}\n{text}");
            // 書き出しは決まった形（同じ入力は同じバイト）
            assert_eq!(text, encode(tool, &presets));
        }
    }

    #[test]
    fn the_text_is_stable_and_names_keep_their_equals_sign() {
        let presets = vec![UserPreset {
            id: 1,
            name: "a=b".into(),
            values: {
                let mut v = defaults(Tool::Gradient);
                v[1] = Value::Choice("sub");
                v
            },
        }];
        assert_eq!(
            encode(Tool::Gradient, &presets),
            "yolupainter-subtools 1\ntool=gradient\npreset.1.name=a=b\npreset.1.shape=linear\npreset.1.end=sub\n"
        );
    }

    #[test]
    fn the_bucket_snap_to_symmetry_ruler_is_kept_in_a_saved_preset_and_is_on_for_files_without_it()
    {
        let at = fields(Tool::Fill)
            .iter()
            .position(|f| f.key == "snap_symmetry")
            .unwrap();
        // 古いファイル（欄が無い）は入
        let old = "yolupainter-subtools 1\ntool=fill\npreset.1.name=x\n";
        assert_eq!(
            decode(Tool::Fill, old).unwrap()[0].values[at],
            Value::Bool(true)
        );
        // 切にして書き出し、読み直しても切
        let mut values = defaults(Tool::Fill);
        values[at] = Value::Bool(false);
        let presets = vec![UserPreset {
            id: 1,
            name: "対称なし".into(),
            values,
        }];
        let text = encode(Tool::Fill, &presets);
        assert!(text.contains("preset.1.snap_symmetry=false"), "{text}");
        assert_eq!(decode(Tool::Fill, &text).unwrap(), presets);
    }

    #[test]
    fn missing_fields_read_as_defaults() {
        let text = "yolupainter-subtools 1\ntool=fill\npreset.3.name=x\npreset.3.tolerance=9\n";
        let got = decode(Tool::Fill, text).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, 3);
        let at = fields(Tool::Fill)
            .iter()
            .position(|f| f.key == "tolerance")
            .unwrap();
        assert_eq!(got[0].values[at], Value::Int(9));
        assert_eq!(got[0].values[0], Value::Bool(false));
    }

    #[test]
    fn broken_files_are_refused_with_a_reason() {
        let head = "yolupainter-subtools 1\ntool=fill\n";
        let cases: Vec<(String, &str)> = vec![
            ("hello".into(), "NotSubTools"),
            ("yolupainter-subtools 2\n".into(), "NewerVersion"),
            (
                format!("{head}preset.1.name=a\npreset.1.nope=1\n"),
                "UnknownKey",
            ),
            (
                format!("{head}preset.1.name=a\npreset.1.name=b\n"),
                "DuplicateKey",
            ),
            (
                format!("{head}preset.1.name=a\npreset.1.tolerance=300\n"),
                "BadValue",
            ),
            (
                format!("{head}preset.1.name=a\npreset.1.tolerance=x\n"),
                "BadValue",
            ),
            (
                format!("{head}preset.1.name=a\npreset.1.by_color=yes\n"),
                "BadValue",
            ),
            (
                format!("{head}preset.1.name=a\npreset.1.kind=sphere\n"),
                "BadValue",
            ),
            (format!("{head}preset.1.tolerance=3\n"), "BadValue"),
            (format!("{head}preset.1.name= \n"), "BadValue"),
            (format!("{head}preset.0.name=a\n"), "UnknownKey"),
            (format!("{head}preset.x.name=a\n"), "UnknownKey"),
            (format!("{head}garbage\n"), "Syntax"),
            (
                "yolupainter-subtools 1\ntool=gradient\n".into(),
                "WrongTool",
            ),
            ("yolupainter-subtools 1\npreset.1.name=a\n".into(), "Syntax"),
            ("yolupainter-subtools 1\n".into(), "WrongTool"),
            (format!("{head}tool=fill\n"), "DuplicateKey"),
        ];
        for (text, kind) in cases {
            let error = decode(Tool::Fill, &text).unwrap_err();
            assert!(
                format!("{error:?}").starts_with(kind),
                "{text:?} -> {error:?}"
            );
            assert!(!error.describe(Lang::Ja).is_empty() && !error.describe(Lang::En).is_empty());
        }
        let many: String = (1..=MAX_USER_PRESETS as u32 + 1)
            .map(|n| format!("preset.{n}.name=p\n"))
            .collect();
        assert!(matches!(
            decode(Tool::Fill, &format!("{head}{many}")),
            Err(StoreError::TooMany)
        ));
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yolu-subtool-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn saved_presets_load_back_and_an_empty_list_removes_the_file() {
        let dir = temp("roundtrip");
        let store = SubToolStore::new(dir.clone());
        let fill = vec![preset(Tool::Fill, 1, "a"), preset(Tool::Fill, 5, "b")];
        store.save(Tool::Fill, &fill).unwrap();
        store
            .save(Tool::Ruler, &[preset(Tool::Ruler, 2, "r")])
            .unwrap();
        let report = load_all(&dir);
        assert!(report.problems.is_empty(), "{:?}", report.problems);
        assert_eq!(report.presets.len(), 2);
        assert!(report.presets.contains(&(Tool::Fill, fill)));
        store.save(Tool::Fill, &[]).unwrap();
        assert!(!store.path_of(Tool::Fill).exists());
        store.save(Tool::Fill, &[]).unwrap(); // もう無くてもよい
        assert_eq!(load_all(&dir).presets.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_bad_file_is_skipped_with_its_name_and_the_others_are_read() {
        let dir = temp("bad");
        let store = SubToolStore::new(dir.clone());
        store
            .save(Tool::Gradient, &[preset(Tool::Gradient, 1, "g")])
            .unwrap();
        std::fs::write(store.path_of(Tool::Fill), "yolupainter-subtools 9\n").unwrap();
        std::fs::write(
            store.path_of(Tool::Shape),
            vec![b'x'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        std::fs::write(
            store.path_of(Tool::Eyedropper),
            "yolupainter-subtools 1\ntool=fill\n",
        )
        .unwrap();
        let report = load_all(&dir);
        assert_eq!(report.presets.len(), 1);
        assert_eq!(report.presets[0].0, Tool::Gradient);
        let mut files: Vec<&str> = report.problems.iter().map(|p| p.file.as_str()).collect();
        files.sort();
        assert_eq!(
            files,
            ["eyedropper.ylsubtool", "fill.ylsubtool", "shape.ylsubtool"]
        );
        let shape = report
            .problems
            .iter()
            .find(|p| p.file == "shape.ylsubtool")
            .unwrap();
        assert!(
            matches!(shape.reason, StoreError::TooLarge),
            "{:?}",
            shape.reason
        );
        // 新しい版のファイルは触らない
        assert_eq!(
            std::fs::read_to_string(store.path_of(Tool::Fill)).unwrap(),
            "yolupainter-subtools 9\n"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_folder_that_does_not_exist_is_an_empty_report_and_a_directory_in_the_way_is_a_problem() {
        let dir = temp("missing");
        let report = load_all(&dir.join("none"));
        assert!(report.presets.is_empty() && report.problems.is_empty());
        std::fs::create_dir(dir.join("fill.ylsubtool")).unwrap();
        let report = load_all(&dir);
        assert_eq!(report.problems.len(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn builtin_ids_are_unique_in_each_tool_and_name_real_fields() {
        for tool in TOOLS {
            let list = builtins(tool);
            let mut ids: Vec<&str> = list.iter().map(|b| b.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), list.len(), "{tool:?}");
            for b in list {
                for (key, value) in b.with {
                    let field = fields(tool)
                        .iter()
                        .find(|f| f.key == *key)
                        .unwrap_or_else(|| panic!("{tool:?} {key}"));
                    let text = text_of(value);
                    assert_eq!(
                        &parse_value(field, &text, key).unwrap(),
                        value,
                        "{tool:?} {}",
                        b.id
                    );
                }
            }
        }
    }

    /// 1 つのプリセットを書いたときの、いちばん長い場合の大きさ（名前は 40 文字がすべて 4 バイト、番号は 10 桁、値はその欄でいちばん長い書き方）。
    fn worst_preset_bytes(tool: Tool) -> usize {
        let prefix = "preset.4294967295.".len();
        let name = prefix + "name=".len() + crate::brushes::MAX_NAME_CHARS * 4 + 1;
        let values: usize = fields(tool)
            .iter()
            .map(|f| {
                let value = match f.kind {
                    Kind::Bool => 5,
                    Kind::Int(min, max) => min.to_string().len().max(max.to_string().len()),
                    // 小数は Rust の表記で指数を使わないので、いちばん小さい正の数が最長（`f32::from_bits(1)` は 48 文字）
                    Kind::Float(..) => 50,
                    Kind::Choice(options) => options.iter().map(|o| o.len()).max().unwrap_or(0),
                };
                prefix + f.key.len() + 1 + value + 1
            })
            .sum();
        name + values
    }

    #[test]
    fn the_largest_number_of_presets_always_fits_in_the_file_limit_for_every_tool() {
        // 欄ごとの「いちばん長い書き方」の見積りが、実際の書き方より短くないこと
        for f in [f32::from_bits(1), -f32::from_bits(1), f32::MAX, -f32::MAX] {
            assert!(text_of(&Value::Float(f)).len() <= 50, "{f}");
        }
        for tool in TOOLS {
            let worst = worst_preset_bytes(tool) * MAX_USER_PRESETS + HEADER.len() + 16;
            assert!(
                worst as u64 <= MAX_FILE_BYTES,
                "{tool:?}: 最長の書き方で {MAX_USER_PRESETS} 個は {worst} バイト（上限 {MAX_FILE_BYTES}）"
            );
        }
    }

    /// 欄ごとに、範囲の中でいちばん長く書ける値。
    fn longest_values(tool: Tool) -> Values {
        fields(tool)
            .iter()
            .map(|f| match f.kind {
                Kind::Bool => Value::Bool(false),
                Kind::Int(min, max) => {
                    if min.to_string().len() > max.to_string().len() {
                        Value::Int(min)
                    } else {
                        Value::Int(max)
                    }
                }
                Kind::Float(min, max) => [min, max, f32::from_bits(1), -f32::from_bits(1)]
                    .into_iter()
                    .filter(|v| (min..=max).contains(v))
                    .map(Value::Float)
                    .max_by_key(|v| text_of(v).len())
                    .unwrap_or(f.default.clone()),
                Kind::Choice(options) => Value::Choice(
                    options
                        .iter()
                        .max_by_key(|o| o.len())
                        .copied()
                        .unwrap_or_default(),
                ),
            })
            .collect()
    }

    #[test]
    fn the_largest_number_of_longest_presets_is_saved_and_read_back_for_every_tool() {
        let dir = temp("largest");
        let store = SubToolStore::new(dir.clone());
        for tool in TOOLS {
            let presets: Vec<UserPreset> = (0..MAX_USER_PRESETS as u32)
                .map(|n| UserPreset {
                    id: u32::MAX - MAX_USER_PRESETS as u32 + 1 + n,
                    name: "\u{20BB7}".repeat(crate::brushes::MAX_NAME_CHARS),
                    values: longest_values(tool),
                })
                .collect();
            store
                .save(tool, &presets)
                .unwrap_or_else(|e| panic!("{tool:?}: {MAX_USER_PRESETS} 個を保存できない: {e:?}"));
            let size = std::fs::metadata(store.path_of(tool)).unwrap().len();
            assert!(size <= MAX_FILE_BYTES, "{tool:?}: {size}");
            let report = load_all(&dir);
            assert!(
                report.problems.is_empty(),
                "{tool:?}: {:?}",
                report.problems
            );
            assert!(
                report.presets.contains(&(tool, presets)),
                "{tool:?}: 読み戻した中身が違う"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
