//! 隠し方のプリセットの保存（設定のフォルダの `hide_presets/`）。プリセット 1 つが 1 ファイル（`hide-<番号>.ylhide`）。
//!
//! 形式は 1 行目が `yolupainter-hide 1`、あとは `key=value` の行（UTF-8、256 KiB まで）: `name=` 名前（1 つ）、`entry=` しきい値・子を含むか
//! （0 か 1）・骨の名前の道（根から `/` 区切り。名前の中の `%`・`/`・`,`・改行は `%XX`）を `,` でつないだもの（何行でも）。骨はモデルの
//! 番号ではなく名前の道で持つので、骨の名前の組が同じモデルへ持ち越せる（合わない項目は、使うときに理由つきで飛ばす。`.ylp` には入れない）。
//! 知らない項目・名前の重なり・範囲を外れた値・新しい版のファイルは、そのファイルを読み飛ばして理由を残す（ほかのファイルは読む）。
//! 書き込みは、一時ファイルへ書いて読み戻して確かめてから、プリセットのファイルを 1 回の置換で確定する（途中で落ちても、前の版か
//! 新しい版のどちらか）。

use std::io;
use std::path::{Path, PathBuf};

use crate::lang::Lang;
use crate::userfiles;

pub const HEADER: &str = "yolupainter-hide 1";
const EXTENSION: &str = "ylhide";
/// 1 ファイルの大きさの上限。
const MAX_FILE_BYTES: u64 = 256 * 1024;
/// プリセットの数の上限（これより多いファイルは読み飛ばす）。
pub const MAX_PRESETS: usize = 256;
/// 1 つのプリセットの項目の数の上限。
pub const MAX_ENTRIES: usize = 1024;
/// 名前の長さの上限（文字数）。
pub const MAX_NAME_CHARS: usize = 64;

/// プリセットの項目（骨の名前の道としきい値・子を含むか）。
#[derive(Clone, Debug, PartialEq)]
pub struct PresetEntry {
    pub path: Vec<String>,
    pub threshold: f32,
    pub children: bool,
}

impl PresetEntry {
    /// 道を `/` でつないだ表示用の文字。
    pub fn path_text(&self) -> String {
        self.path.join("/")
    }
}

/// 名前つきの隠し方。
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub id: u32,
    pub name: String,
    pub entries: Vec<PresetEntry>,
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
    /// 項目が無い。
    Empty,
    /// 書いたファイルを読み戻したら、書いた中身と違った。
    Mismatch,
    TooMany,
}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl From<userfiles::FileError> for StoreError {
    fn from(e: userfiles::FileError) -> Self {
        match e {
            userfiles::FileError::Io(e) => StoreError::Io(e),
            userfiles::FileError::TooLarge => StoreError::TooLarge,
            userfiles::FileError::NotText => StoreError::NotAPreset,
            userfiles::FileError::Mismatch => StoreError::Mismatch,
        }
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
                .pick("隠し方のファイルではありません", "Not a hide preset file")
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
            StoreError::Empty => lang.pick("項目がありません", "No entries").into(),
            StoreError::Mismatch => lang
                .pick(
                    "書いた中身を読み戻せません",
                    "Written file does not read back",
                )
                .into(),
            StoreError::TooMany => lang
                .pick("プリセットが多すぎます", "Too many presets")
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

/// 隠し方のプリセットの一覧（設定のフォルダが分からないときは、保存せずにこの起動の間だけ持つ）。
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
        let loaded = userfiles::load_numbered(
            &dir,
            file_id,
            MAX_PRESETS,
            read_file,
            |(name, _)| name.as_str(),
            || StoreError::TooMany,
            || StoreError::DuplicateName,
        );
        self.items = loaded
            .items
            .into_iter()
            .map(|(id, (name, entries))| Preset { id, name, entries })
            .collect();
        self.problems = loaded
            .problems
            .into_iter()
            .map(|(file, error)| Problem { file, error })
            .collect();
        self.next_id = loaded.next_id;
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
    /// 同じ）を超えないように、番号の分だけ元の名前を切る。
    pub fn unique_name(&self, base: &str) -> String {
        let base: String = base.trim().chars().take(MAX_NAME_CHARS).collect();
        if !self.items.iter().any(|p| p.name == base) {
            return base;
        }
        (2..)
            .map(|n| {
                let suffix = format!(" {n}");
                let keep = MAX_NAME_CHARS.saturating_sub(suffix.chars().count());
                let stem: String = base.chars().take(keep).collect();
                format!("{}{suffix}", stem.trim_end())
            })
            .find(|name| !self.items.iter().any(|p| &p.name == name))
            .unwrap_or(base)
    }

    /// 新しいプリセットを保存する（名前が重なるときは番号を付ける）。保存できたらその番号。
    pub fn add(&mut self, name: &str, entries: Vec<PresetEntry>) -> Result<u32, StoreError> {
        if name.trim().is_empty() {
            return Err(StoreError::BadValue("name".into()));
        }
        if entries.is_empty() {
            return Err(StoreError::Empty);
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

    /// プリセットを消す（ファイルも。利用者が消すと選んだものだけ）。
    pub fn remove(&mut self, id: u32) -> Result<(), StoreError> {
        let Some(at) = self.items.iter().position(|p| p.id == id) else {
            return Ok(());
        };
        if let Some(dir) = &self.dir {
            userfiles::remove(&path_of(dir, id))?;
        }
        self.items.remove(at);
        Ok(())
    }
}

fn file_name(id: u32) -> String {
    format!("hide-{id}.{EXTENSION}")
}

fn path_of(dir: &Path, id: u32) -> PathBuf {
    dir.join(file_name(id))
}

/// `hide-<番号>.ylhide` の番号。
fn file_id(path: &Path) -> Option<u32> {
    let name = path.file_name()?.to_str()?;
    let stem = name.strip_suffix(&format!(".{EXTENSION}"))?;
    stem.strip_prefix("hide-")?.parse().ok()
}

// ───────── 形式 ─────────

/// 名前の中の `%`・`/`・`,`・改行を `%XX` にする（ポーズのプリセットの形式も同じ決まり）。
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '%' => out.push_str("%25"),
            '/' => out.push_str("%2F"),
            ',' => out.push_str("%2C"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            c => out.push(c),
        }
    }
    out
}

/// `escape` の逆（`%XX` が正しくない・UTF-8 でないときは None）。
pub(crate) fn unescape(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = text.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// ファイルの中身。
pub fn render(name: &str, entries: &[PresetEntry]) -> String {
    let mut text = format!("{HEADER}\nname={}\n", escape(name));
    for e in entries {
        let path: Vec<String> = e.path.iter().map(|c| escape(c)).collect();
        text += &format!(
            "entry={},{},{}\n",
            e.threshold,
            u8::from(e.children),
            path.join("/")
        );
    }
    text
}

/// ファイルの中身を読む（名前と項目）。
pub fn parse(text: &str) -> Result<(String, Vec<PresetEntry>), StoreError> {
    let mut lines = text.lines();
    match lines.next() {
        Some(HEADER) => {}
        Some(first) if first.starts_with("yolupainter-hide ") => {
            return Err(StoreError::NewerVersion(first.to_owned()));
        }
        _ => return Err(StoreError::NotAPreset),
    }
    let mut name: Option<String> = None;
    let mut entries = Vec::new();
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
            "entry" => {
                if entries.len() >= MAX_ENTRIES {
                    return Err(StoreError::TooMany);
                }
                let bad = || StoreError::BadValue("entry".into());
                let mut parts = value.splitn(3, ',');
                let threshold: f32 = parts.next().and_then(|v| v.parse().ok()).ok_or_else(bad)?;
                let children = match parts.next() {
                    Some("0") => false,
                    Some("1") => true,
                    _ => return Err(bad()),
                };
                let path_text = parts.next().ok_or_else(bad)?;
                if !threshold.is_finite() || !(0.0..=1.0).contains(&threshold) {
                    return Err(bad());
                }
                let path: Vec<String> = path_text
                    .split('/')
                    .map(unescape)
                    .collect::<Option<_>>()
                    .ok_or_else(bad)?;
                if path.iter().any(|c| c.is_empty()) {
                    return Err(bad());
                }
                entries.push(PresetEntry {
                    path,
                    threshold,
                    children,
                });
            }
            other => return Err(StoreError::UnknownKey(other.to_owned())),
        }
    }
    let name = name.ok_or_else(|| StoreError::BadValue("name".into()))?;
    if entries.is_empty() {
        return Err(StoreError::Empty);
    }
    Ok((name, entries))
}

fn read_file(path: &Path) -> Result<(String, Vec<PresetEntry>), StoreError> {
    parse(&userfiles::read_checked(path, MAX_FILE_BYTES)?)
}

/// 一時ファイルへ書き、読み戻して確かめてから、プリセットのファイルへ 1 回の置換で確定する。
fn write_file(dir: &Path, preset: &Preset) -> Result<(), StoreError> {
    let text = render(&preset.name, &preset.entries);
    userfiles::write_text(&path_of(dir, preset.id), &text, MAX_FILE_BYTES, |read| {
        parse(read).is_ok_and(|(name, entries)| name == preset.name && entries == preset.entries)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yolu-hide-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn entry(path: &[&str], threshold: f32, children: bool) -> PresetEntry {
        PresetEntry {
            path: path.iter().map(|s| s.to_string()).collect(),
            threshold,
            children,
        }
    }

    #[test]
    fn a_preset_round_trips_through_the_file_text_including_awkward_names() {
        let entries = vec![
            entry(&["腰", "背骨", "頭"], 0.5, true),
            entry(&["Armature/Root", "a,b", "100%\n改行"], 0.25, false),
        ];
        let text = render("髪・耳 / 100%", &entries);
        assert!(text.starts_with("yolupainter-hide 1\n"));
        assert_eq!(text.lines().count(), 4, "改行を含む名前も 1 行: {text}");
        let (name, back) = parse(&text).unwrap();
        assert_eq!(name, "髪・耳 / 100%");
        assert_eq!(back, entries);
    }

    #[test]
    fn broken_files_are_refused_with_a_reason() {
        let good = render("名前", &[entry(&["根"], 0.5, true)]);
        assert!(matches!(parse(""), Err(StoreError::NotAPreset)));
        assert!(matches!(
            parse("yolupainter-brush 1\n"),
            Err(StoreError::NotAPreset)
        ));
        assert!(matches!(
            parse("yolupainter-hide 2\nname=x\n"),
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
        for bad in [
            "entry=2,1,根",
            "entry=-0.1,1,根",
            "entry=NaN,1,根",
            "entry=0.5,2,根",
            "entry=0.5,1,",
            "entry=0.5,1,根//子",
            "entry=0.5,1",
            "entry=0.5,1,%ZZ",
        ] {
            assert!(
                matches!(
                    parse(&format!("yolupainter-hide 1\nname=名前\n{bad}\n")),
                    Err(StoreError::BadValue(_))
                ),
                "{bad}"
            );
        }
        assert!(matches!(
            parse("yolupainter-hide 1\nname=名前\n"),
            Err(StoreError::Empty)
        ));
        assert!(matches!(
            parse("yolupainter-hide 1\nentry=0.5,1,根\n"),
            Err(StoreError::BadValue(_))
        ));
        let long = "あ".repeat(MAX_NAME_CHARS + 1);
        assert!(matches!(
            parse(&format!(
                "yolupainter-hide 1\nname={long}\nentry=0.5,1,根\n"
            )),
            Err(StoreError::BadValue(_))
        ));
    }

    #[test]
    fn saving_writes_a_file_and_loading_reads_it_back_in_order() {
        let dir = dir("save");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        assert!(presets.items().is_empty());
        let a = presets
            .add("髪", vec![entry(&["腰", "頭"], 0.5, true)])
            .unwrap();
        let b = presets
            .add("服", vec![entry(&["腰", "胸"], 0.3, false)])
            .unwrap();
        assert!(b > a);
        assert!(dir.join(format!("hide-{a}.ylhide")).exists());
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
        assert!(!dir.join(format!("hide-{b}.ylhide")).exists());
        let c = again.add("靴", vec![entry(&["腰"], 0.5, true)]).unwrap();
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
        let e = || vec![entry(&["根"], 0.5, true)];
        let a = presets.add("隠し方", e()).unwrap();
        let b = presets.add("隠し方", e()).unwrap();
        let c = presets.add(" 隠し方 ", e()).unwrap();
        let names: Vec<&str> = presets.items().iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["隠し方", "隠し方 2", "隠し方 3"]);
        assert!(a != b && b != c);
        assert!(matches!(
            presets.add("  ", e()),
            Err(StoreError::BadValue(_))
        ));
        assert!(matches!(
            presets.add("空", Vec::new()),
            Err(StoreError::Empty)
        ));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_numbered_name_stays_within_the_name_limit_and_reads_back() {
        let dir = dir("longname");
        let mut presets = Presets::default();
        presets.attach(dir.clone());
        let e = || vec![entry(&["根"], 0.5, true)];
        let long = "あ".repeat(MAX_NAME_CHARS);
        let a = presets.add(&long, e()).unwrap();
        let b = presets.add(&long, e()).unwrap();
        let c = presets.add(&long, e()).unwrap();
        for p in presets.items() {
            assert!(p.name.chars().count() <= MAX_NAME_CHARS, "{}", p.name);
        }
        assert_eq!(presets.get(a).unwrap().name, long);
        assert!(presets.get(b).unwrap().name.ends_with("あ 2"));
        assert!(presets.get(c).unwrap().name.ends_with("あ 3"));
        // 保存した 3 つとも、読み戻せる（読み戻しは 64 文字を超える名前を断る）
        let mut again = Presets::default();
        again.attach(dir.clone());
        assert!(again.problems.is_empty(), "{:?}", again.problems);
        assert_eq!(again.items(), presets.items());
        // 64 文字より長い名前は、切ってから番号を付ける（元の名前が切られても重なりを避ける）
        let longer = format!("{long}いう");
        let d = presets.add(&longer, e()).unwrap();
        assert_eq!(presets.get(d).unwrap().name.chars().count(), MAX_NAME_CHARS);
        assert!(presets.get(d).unwrap().name.ends_with("あ 4"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn too_many_presets_or_entries_are_refused_on_load_parse_and_add() {
        // attach: 上限を超えたファイルは読み飛ばして理由を残し、上限までは読む
        let dir = dir("toomany");
        std::fs::create_dir_all(&dir).unwrap();
        for id in 1..=(MAX_PRESETS + 1) {
            std::fs::write(
                dir.join(format!("hide-{id}.ylhide")),
                render(&format!("p{id}"), &[entry(&["根"], 0.5, true)]),
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
            format!("hide-{}.ylhide", MAX_PRESETS + 1)
        );
        assert!(
            dir.join(format!("hide-{}.ylhide", MAX_PRESETS + 1))
                .exists(),
            "読み飛ばしたファイルには触らない"
        );
        // add: 上限いっぱいのところへ足すと断る（ファイルも一覧も増えない）
        let err = presets.add("あふれ", vec![entry(&["根"], 0.5, true)]);
        assert!(matches!(err, Err(StoreError::TooMany)));
        assert_eq!(presets.items().len(), MAX_PRESETS);
        let files = std::fs::read_dir(&dir).unwrap().flatten().count();
        assert_eq!(files, MAX_PRESETS + 1);
        std::fs::remove_dir_all(dir).unwrap();

        // メモリだけでも同じ上限
        let mut memory = Presets::default();
        for i in 0..MAX_PRESETS {
            memory
                .add(&format!("m{i}"), vec![entry(&["根"], 0.5, true)])
                .unwrap();
        }
        assert!(matches!(
            memory.add("あふれ", vec![entry(&["根"], 0.5, true)]),
            Err(StoreError::TooMany)
        ));
        // 項目が多すぎる add
        let mut few = Presets::default();
        let many: Vec<PresetEntry> = (0..=MAX_ENTRIES)
            .map(|_| entry(&["根"], 0.5, true))
            .collect();
        assert!(matches!(few.add("多い", many), Err(StoreError::TooMany)));
        assert!(few.items().is_empty());
        let exact: Vec<PresetEntry> = (0..MAX_ENTRIES)
            .map(|_| entry(&["根"], 0.5, true))
            .collect();
        assert!(few.add("ちょうど", exact).is_ok());

        // parse: 項目の数の上限
        let line = "entry=0.5,1,根\n";
        let text = |n: usize| format!("{HEADER}\nname=x\n{}", line.repeat(n));
        assert_eq!(parse(&text(MAX_ENTRIES)).unwrap().1.len(), MAX_ENTRIES);
        assert!(matches!(
            parse(&text(MAX_ENTRIES + 1)),
            Err(StoreError::TooMany)
        ));
        let too_many = StoreError::TooMany;
        assert!(!too_many.describe(Lang::Ja).is_empty() && too_many.describe(Lang::En).is_ascii());
    }

    #[test]
    fn unreadable_files_are_skipped_with_a_reason_and_the_rest_load() {
        let dir = dir("problems");
        std::fs::create_dir_all(&dir).unwrap();
        let good = render("良い", &[entry(&["根"], 0.5, true)]);
        std::fs::write(dir.join("hide-1.ylhide"), good).unwrap();
        std::fs::write(dir.join("hide-2.ylhide"), "yolupainter-hide 9\n").unwrap();
        std::fs::write(dir.join("hide-3.ylhide"), "壊れた").unwrap();
        std::fs::write(
            dir.join("hide-4.ylhide"),
            vec![b'a'; MAX_FILE_BYTES as usize + 1],
        )
        .unwrap();
        std::fs::write(dir.join("memo.txt"), "関係ないファイル").unwrap();
        std::fs::write(
            dir.join("hide-5.ylhide"),
            render("良い", &[entry(&["根"], 0.5, true)]),
        )
        .unwrap();
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
                ("hide-2.ylhide".to_string(), "newer"),
                ("hide-3.ylhide".to_string(), "not"),
                ("hide-4.ylhide".to_string(), "large"),
                ("hide-5.ylhide".to_string(), "dup"),
            ]
        );
        // 読めなかったファイルには触らない
        assert!(dir.join("hide-2.ylhide").exists());
        // 番号は最大の続き（読めなかったファイルの番号も使わない）
        let id = presets
            .add("新しい", vec![entry(&["根"], 0.5, true)])
            .unwrap();
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
            .add("メモリだけ", vec![entry(&["根"], 0.5, true)])
            .unwrap();
        assert_eq!(presets.get(id).unwrap().name, "メモリだけ");
        presets.remove(id).unwrap();
        assert!(presets.items().is_empty());
        presets.remove(999).unwrap();
    }

    #[test]
    fn a_write_that_cannot_happen_leaves_the_list_unchanged() {
        let dir = dir("blocked");
        std::fs::create_dir_all(&dir).unwrap();
        // フォルダの場所にファイルがあって作れない
        let blocked = dir.join("hide_presets");
        std::fs::write(&blocked, "ファイル").unwrap();
        let mut presets = Presets::default();
        presets.attach(blocked.clone());
        let err = presets.add("書けない", vec![entry(&["根"], 0.5, true)]);
        assert!(matches!(err, Err(StoreError::Io(_))));
        assert!(presets.items().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
