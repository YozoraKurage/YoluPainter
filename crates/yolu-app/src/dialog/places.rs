//! ファイルを選ぶウィンドウの始まりの場所。
//!
//! 親を渡す口（`dialog::file`）が、入り口の種類（`Place`）ごとに前に使った場所を覚え、次に開くときの始まりの場所にする。何も渡さないと OS が決める
//! 場所から始まる（環境によっては「/」）ので、始まりの場所は必ずここで決める。
//!
//! 決め方（`start_folder`。ウィンドウを開かない純粋な関数）:
//! - 開く・取り込み・書き出し（`Rule::Open`）: 前にその種類で使った場所 → 今の文書の .ylp のあるフォルダ → OS の書類のフォルダ。
//! - 保存・別名で保存（`Rule::Save`）: 退避を開いた文書の元のフォルダ（`AppState::save_folder`）→ 今の文書の .ylp のあるフォルダ →
//!   前にその種類で使った場所 → OS の書類のフォルダ。
//!
//! どちらも、今ある（フォルダとして開ける）場所だけを使う（無くなった場所は飛ばして次へ）。
//!
//! 覚えた場所は、設定のファイル（`settings.conf`）と同じフォルダの `places.conf` に `種類=絶対パス` の 1 行ずつで残す（端末ごと）。設定のファイルは
//! 4 KiB までの決まりで、パスを何本も足すと設定を保存できなくなるので別のファイルにした。ファイルがまだ無いときは空から始める。読めないファイル
//! （UTF-8 でない・大きすぎる・権限など）は、書き換えず（`recovery.conf` と同じ）、そのアプリを開いている間だけ覚える。読めない行は読み飛ばして、ほかの行は
//! 生かす。書けなければ（権限・ディスク）、開いている間だけ覚える。どの場合もウィンドウの選びは止めない。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// ファイルを選ぶ入り口の種類（前に使った場所を種類ごとに覚える）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Place {
    /// プロジェクト（.ylp）を開く・別名で保存。
    Project,
    /// 配布用に保存。
    Distribute,
    /// 3D のモデル（FBX）を開く。
    Model,
    /// PSD の取り込み。
    PsdImport,
    /// PSD の書き出し。
    PsdExport,
    /// 画像の取り込み（ステンシル・塗りつぶしの画像）。
    ImageImport,
    /// 画像の書き出し（チャンネルの PNG・テンプレートの書き出し先）。
    ImageExport,
    /// ブラシの取り込み。
    Brush,
    /// フォントのファイル。
    Font,
    /// キーの設定の読み書き。
    Keys,
    /// アセット・ライブラリ（スマート素材・画像）の読み書き。
    Assets,
    /// カラーセット（GPL・ACO）の読み書き。
    ColorSet,
    /// 設定の置き場（ライブラリ・キャッシュ・CLIP STUDIO のフォルダ）を選ぶ。
    Settings,
}

impl Place {
    pub const ALL: [Place; 13] = [
        Place::Project,
        Place::Distribute,
        Place::Model,
        Place::PsdImport,
        Place::PsdExport,
        Place::ImageImport,
        Place::ImageExport,
        Place::Brush,
        Place::Font,
        Place::Keys,
        Place::Assets,
        Place::ColorSet,
        Place::Settings,
    ];

    /// `places.conf` のキー。
    pub fn key(self) -> &'static str {
        match self {
            Place::Project => "project",
            Place::Distribute => "distribute",
            Place::Model => "model",
            Place::PsdImport => "psd_import",
            Place::PsdExport => "psd_export",
            Place::ImageImport => "image_import",
            Place::ImageExport => "image_export",
            Place::Brush => "brush",
            Place::Font => "font",
            Place::Keys => "keys",
            Place::Assets => "assets",
            Place::ColorSet => "color_set",
            Place::Settings => "settings",
        }
    }
}

/// 始まりの場所の決め方。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// 開く・取り込み・書き出し: 前に使った場所を先にする。
    Open,
    /// 保存・別名で保存: 今の文書の場所を先にする。
    Save,
}

/// 始まりの場所の材料。
#[derive(Clone, Copy, Debug, Default)]
pub struct Sources<'a> {
    /// 退避を開いた文書の保存の始まり（元の .ylp のフォルダ。`Rule::Save` だけが先に使う）。
    pub preferred: Option<&'a Path>,
    /// 今の文書の .ylp のあるフォルダ。
    pub document: Option<&'a Path>,
    /// 前にその種類で使った場所。
    pub remembered: Option<&'a Path>,
    /// OS の書類のフォルダ。
    pub documents: Option<&'a Path>,
}

/// 始まりの場所を決める。今あるフォルダ（`is_dir`）の中で、`rule` の順の最初のもの。どれも無ければ None（OS に任せる）。
pub fn start_folder(
    rule: Rule,
    sources: &Sources<'_>,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    let order: [Option<&Path>; 4] = match rule {
        Rule::Open => [
            sources.remembered,
            sources.document,
            sources.documents,
            None,
        ],
        Rule::Save => [
            sources.preferred,
            sources.document,
            sources.remembered,
            sources.documents,
        ],
    };
    order
        .into_iter()
        .flatten()
        .find(|p| p.is_absolute() && is_dir(p))
        .map(Path::to_path_buf)
}

/// 覚えた場所（種類ごとのフォルダ）と、残すファイル。
#[derive(Clone, Debug, Default)]
pub struct Places {
    remembered: BTreeMap<Place, PathBuf>,
    /// 残すファイル（None なら、このアプリを開いている間だけ覚える。試験の状態は設定のフォルダに触れない）。
    path: Option<PathBuf>,
}

/// 読むファイルの大きさの上限（種類 13 本 × パス 1 本）。
const MAX_FILE_BYTES: u64 = 64 * 1024;

impl Places {
    /// 設定のフォルダの `places.conf`（設定のフォルダが分からなければ None）。
    pub fn default_path() -> Option<PathBuf> {
        crate::settings::path().map(|p| p.with_file_name("places.conf"))
    }

    /// ファイルから覚えた場所を読む。ファイルが無ければ空から始める（場所を覚えるときに作る）。読めない（UTF-8 でない・大きすぎる・権限など）ときは、
    /// ファイルを残す先から外し（`path` を None に）、開いている間だけ覚える（読めなかったファイルを書き換えない）。読めない行は読み飛ばす。
    pub fn load(path: Option<PathBuf>) -> Places {
        let Some(file) = path else {
            return Places::default();
        };
        match read(&file) {
            Ok(text) => Places {
                remembered: parse(&text),
                path: Some(file),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Places {
                remembered: BTreeMap::new(),
                path: Some(file),
            },
            Err(_) => Places::default(),
        }
    }

    /// 種類の前に使った場所。
    pub fn get(&self, place: Place) -> Option<&Path> {
        self.remembered.get(&place).map(PathBuf::as_path)
    }

    /// 種類の場所を覚える（フォルダを絶対パスで。相対・改行を含むパスは覚えない）。変わったときだけファイルに書く。変わったか。
    pub fn remember(&mut self, place: Place, folder: &Path) -> bool {
        if !usable(folder) || self.get(place) == Some(folder) {
            return false;
        }
        self.remembered.insert(place, folder.to_path_buf());
        if let Some(path) = &self.path {
            // 書けなくても、開いている間は覚えている
            let _ = save(path, &self.remembered);
        }
        true
    }
}

/// 覚えてよいパスか（絶対パスで、1 行に書ける）。
fn usable(folder: &Path) -> bool {
    folder.is_absolute() && !folder.to_string_lossy().contains(['\n', '\r'])
}

fn read(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let file = std::fs::File::open(path)?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text)?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "places.conf too large",
        ));
    }
    Ok(text)
}

fn parse(text: &str) -> BTreeMap<Place, PathBuf> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let Some(place) = Place::ALL.into_iter().find(|p| p.key() == key) else {
            continue;
        };
        let folder = PathBuf::from(value);
        if usable(&folder) {
            out.insert(place, folder);
        }
    }
    out
}

fn render(remembered: &BTreeMap<Place, PathBuf>) -> String {
    remembered
        .iter()
        .map(|(place, folder)| format!("{}={}\n", place.key(), folder.display()))
        .collect()
}

fn save(path: &Path, remembered: &BTreeMap<Place, PathBuf>) -> std::io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "directory missing")
    })?;
    std::fs::create_dir_all(parent)?;
    yolu_io::atomic::replace_bytes(path, render(remembered).as_bytes())
}

/// OS の書類のフォルダ（分からなければ None。決めるのは最初の 1 回だけ）。
pub fn documents_folder() -> Option<PathBuf> {
    static FOLDER: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOLDER.get_or_init(os_documents_folder).clone()
}

#[cfg(windows)]
fn os_documents_folder() -> Option<PathBuf> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    // SAFETY: 書類のフォルダの道を OS から受け取り、文字列にしてから OS の決まりどおり解放する。
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_Documents, KF_FLAG_DEFAULT, None).ok()?;
        let folder = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.as_ptr() as *const _));
        folder
    }
}

#[cfg(not(windows))]
fn os_documents_folder() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".config"));
    let user_dirs = std::fs::read_to_string(config.join("user-dirs.dirs")).ok();
    Some(unix_documents(&home, user_dirs.as_deref()))
}

/// 書類のフォルダ: `user-dirs.dirs` の `XDG_DOCUMENTS_DIR`（`$HOME` を家のフォルダに置き換える）、無ければ家のフォルダの `Documents`。
#[cfg_attr(windows, allow(dead_code))]
fn unix_documents(home: &Path, user_dirs: Option<&str>) -> PathBuf {
    let from_file = user_dirs.and_then(|text| {
        text.lines().find_map(|line| {
            let value = line.trim().strip_prefix("XDG_DOCUMENTS_DIR=")?;
            let value = value.trim().trim_matches('"');
            let replaced = match value.strip_prefix("$HOME") {
                Some(rest) => format!("{}{rest}", home.display()),
                None => value.to_owned(),
            };
            let folder = PathBuf::from(replaced);
            // 家のフォルダそのものを指す設定（書類のフォルダを持たない）は使わない
            (folder.is_absolute() && folder != home).then_some(folder)
        })
    });
    from_file.unwrap_or_else(|| home.join("Documents"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists(list: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p: &Path| list.iter().any(|d| Path::new(d) == p)
    }

    fn sources<'a>(
        preferred: Option<&'a str>,
        document: Option<&'a str>,
        remembered: Option<&'a str>,
        documents: Option<&'a str>,
    ) -> Sources<'a> {
        Sources {
            preferred: preferred.map(Path::new),
            document: document.map(Path::new),
            remembered: remembered.map(Path::new),
            documents: documents.map(Path::new),
        }
    }

    const ALL: &[&str] = &["/d/saved", "/d/doc", "/d/last", "/d/documents"];

    #[test]
    fn opening_starts_where_that_kind_was_last_used_then_the_document_then_the_documents_folder() {
        let all = sources(None, Some("/d/doc"), Some("/d/last"), Some("/d/documents"));
        assert_eq!(
            start_folder(Rule::Open, &all, &exists(ALL)),
            Some(PathBuf::from("/d/last"))
        );
        // 前の場所が無くなっていれば、文書のフォルダ
        let gone = exists(&["/d/doc", "/d/documents"]);
        assert_eq!(
            start_folder(Rule::Open, &all, &gone),
            Some(PathBuf::from("/d/doc"))
        );
        // 文書も無ければ、OS の書類のフォルダ
        let only = exists(&["/d/documents"]);
        assert_eq!(
            start_folder(Rule::Open, &all, &only),
            Some(PathBuf::from("/d/documents"))
        );
        // どれも無ければ OS に任せる
        assert_eq!(start_folder(Rule::Open, &all, &exists(&[])), None);
        // 前に使った場所が無い（初めて）なら、文書のフォルダ
        let fresh = sources(None, Some("/d/doc"), None, Some("/d/documents"));
        assert_eq!(
            start_folder(Rule::Open, &fresh, &exists(ALL)),
            Some(PathBuf::from("/d/doc"))
        );
        // 退避の元のフォルダは、開く・取り込み・書き出しでは使わない
        let with_saved = sources(Some("/d/saved"), None, None, Some("/d/documents"));
        assert_eq!(
            start_folder(Rule::Open, &with_saved, &exists(ALL)),
            Some(PathBuf::from("/d/documents"))
        );
    }

    #[test]
    fn saving_starts_in_the_saved_origin_then_the_document_then_the_last_place_then_the_documents_folder(
    ) {
        let all = sources(
            Some("/d/saved"),
            Some("/d/doc"),
            Some("/d/last"),
            Some("/d/documents"),
        );
        assert_eq!(
            start_folder(Rule::Save, &all, &exists(ALL)),
            Some(PathBuf::from("/d/saved")),
            "退避を開いた文書の元のフォルダが先"
        );
        let no_saved = sources(None, Some("/d/doc"), Some("/d/last"), Some("/d/documents"));
        assert_eq!(
            start_folder(Rule::Save, &no_saved, &exists(ALL)),
            Some(PathBuf::from("/d/doc")),
            "今の文書のフォルダ（前に使った場所より先）"
        );
        let unsaved = sources(None, None, Some("/d/last"), Some("/d/documents"));
        assert_eq!(
            start_folder(Rule::Save, &unsaved, &exists(ALL)),
            Some(PathBuf::from("/d/last")),
            "文書がまだ保存されていなければ前に使った場所"
        );
        assert_eq!(
            start_folder(Rule::Save, &unsaved, &exists(&["/d/documents"])),
            Some(PathBuf::from("/d/documents"))
        );
        // 無くなった場所は飛ばして次へ
        assert_eq!(
            start_folder(Rule::Save, &all, &exists(&["/d/last", "/d/documents"])),
            Some(PathBuf::from("/d/last"))
        );
        // 相対パスは使わない
        let relative = sources(None, Some("doc"), None, None);
        assert_eq!(start_folder(Rule::Save, &relative, &|_| true), None);
    }

    #[test]
    fn remembered_places_are_read_and_written_one_line_per_kind_and_bad_lines_are_dropped() {
        let mut map = BTreeMap::new();
        map.insert(Place::Model, PathBuf::from("/m/models"));
        map.insert(Place::PsdExport, PathBuf::from("/p/out"));
        let text = render(&map);
        assert_eq!(text, "model=/m/models\npsd_export=/p/out\n");
        assert_eq!(parse(&text), map);
        // 知らないキー・相対パス・= の無い行・空は読み飛ばし、ほかの行は生かす
        let messy = "future=/x\nmodel=relative/dir\nnoequals\nfont=\nbrush=/b\n";
        let read = parse(messy);
        assert_eq!(read.len(), 1);
        assert_eq!(read.get(&Place::Brush), Some(&PathBuf::from("/b")));
        // 種類のキーは重ならない
        let keys: std::collections::BTreeSet<_> = Place::ALL.iter().map(|p| p.key()).collect();
        assert_eq!(keys.len(), Place::ALL.len());
    }

    #[test]
    fn remembering_writes_only_a_change_and_only_a_usable_folder() {
        let dir = std::env::temp_dir().join(format!("yolu-places-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("places.conf");
        let mut places = Places::load(Some(file.clone()));
        assert!(places.get(Place::Model).is_none());
        assert!(!file.exists(), "読んだだけでは触らない");
        assert!(places.remember(Place::Model, &dir));
        assert!(file.exists());
        assert!(
            !places.remember(Place::Model, &dir),
            "同じ場所は書き直さない"
        );
        assert!(!places.remember(Place::Brush, Path::new("relative")));
        assert!(places.get(Place::Brush).is_none());
        // 開き直しても残る
        let again = Places::load(Some(file.clone()));
        assert_eq!(again.get(Place::Model), Some(dir.as_path()));
        // ファイルの無い状態（試験・設定のフォルダが分からない）でも、開いている間は覚える
        let mut memory = Places::load(None);
        assert!(memory.remember(Place::Font, &dir));
        assert_eq!(memory.get(Place::Font), Some(dir.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unreadable_file_is_never_overwritten_and_places_are_kept_for_this_run_only() {
        let dir = std::env::temp_dir().join(format!("yolu-places-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cases: [(&str, Vec<u8>); 2] = [
            (
                "not-utf8.conf",
                vec![0xff, 0xfe, b'm', b'o', b'd', b'e', b'l'],
            ),
            ("too-big.conf", vec![b'a'; MAX_FILE_BYTES as usize + 1]),
        ];
        for (name, bytes) in cases {
            let file = dir.join(name);
            std::fs::write(&file, &bytes).unwrap();
            let mut places = Places::load(Some(file.clone()));
            assert!(places.get(Place::Model).is_none(), "{name}");
            assert!(
                places.remember(Place::Model, &dir),
                "{name}: 開いている間は覚える"
            );
            assert_eq!(places.get(Place::Model), Some(dir.as_path()));
            assert_eq!(std::fs::read(&file).unwrap(), bytes, "{name}: 書き換えない");
        }
        // 無いファイルは空から始めて、覚えたときに作る
        let missing = dir.join("missing.conf");
        let mut places = Places::load(Some(missing.clone()));
        assert!(places.remember(Place::Font, &dir));
        assert!(missing.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_documents_folder_follows_user_dirs_and_falls_back_to_documents_under_home() {
        let home = Path::new("/home/u");
        assert_eq!(
            unix_documents(home, None),
            PathBuf::from("/home/u/Documents")
        );
        let dirs =
            "# comment\nXDG_DESKTOP_DIR=\"$HOME/Desktop\"\nXDG_DOCUMENTS_DIR=\"$HOME/書類\"\n";
        assert_eq!(
            unix_documents(home, Some(dirs)),
            PathBuf::from("/home/u/書類")
        );
        let absolute = "XDG_DOCUMENTS_DIR=\"/data/docs\"\n";
        assert_eq!(
            unix_documents(home, Some(absolute)),
            PathBuf::from("/data/docs")
        );
        // 家のフォルダそのものを指す設定（書類のフォルダを持たない）は使わない
        for home_only in [
            "XDG_DOCUMENTS_DIR=\"$HOME\"\n",
            "XDG_DOCUMENTS_DIR=\"$HOME/\"\n",
        ] {
            assert_eq!(
                unix_documents(home, Some(home_only)),
                PathBuf::from("/home/u/Documents")
            );
        }
    }
}
