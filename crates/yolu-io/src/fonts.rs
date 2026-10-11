//! OS に入っているフォントの一覧（`fontdb` でフォントのフォルダをなめる）と、テキストレイヤーのフォントを探す決まり。
//!
//! 一覧を作るのは遅い（フォントのファイルを全部開く）ので、画面は別のスレッドで [`system`] を呼び、できた物を [`loaded`] で受ける。
//! 一覧はプロセスに 1 つで、起動中に入れたフォントはアプリを開き直すまで出ない。Linux は fontconfig の設定を読まず、決まったフォルダ
//! （`/usr/share/fonts`・`/usr/local/share/fonts`・`~/.fonts`・`~/.local/share/fonts`）だけを見る。
//!
//! テキストレイヤーのフォントを探す順（[`find`]）: 覚えた道のファイルの中身が SHA-256 と同じなら、それが同じフォント。違う・無いときは
//! OS のフォントを PostScript 名 → ファミリー名と太さ（と斜体か）で探し、最後に覚えた道のファイル。覚えた道が絶対の道のときだけ使う
//! （配布用の写しはフォントの場所をファイル名だけにするので、ファイル名だけ・相対の道は、今の作業フォルダーの別のファイルを拾わないよう使わない）。
//! 見つけた中身の SHA-256 が違えば [`Lookup::Different`]（画面は「フォントが違います」と知らせ、描き直すときは見つけたフォントで描く）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use yolu_core::text::{self, FontDescription, TextFont};

/// フォントのファイルの大きさの上限（フォントの束でも収まる大きさ）。
pub const MAX_FONT_FILE_BYTES: u64 = 256 << 20;

/// OS のフォント 1 つ（束の中の 1 つ）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFace {
    pub path: PathBuf,
    /// 束（.ttc）の中の番号。
    pub index: u32,
    pub description: FontDescription,
}

impl SystemFace {
    /// 画面のスタイルの名前（フォントのスタイルの名前。無ければ太さと斜体から）。
    pub fn style(&self) -> String {
        let d = &self.description;
        if d.style.is_empty() {
            weight_name(d.names.weight, d.names.italic)
        } else {
            d.style.clone()
        }
    }
}

/// 太さと斜体からのスタイルの名前（フォントにスタイルの名前が無いとき）。
pub fn weight_name(weight: u16, italic: bool) -> String {
    let name = match weight {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    };
    match (italic, name) {
        (true, "Regular") => "Italic".into(),
        (true, n) => format!("{n} Italic"),
        (false, n) => n.into(),
    }
}

/// ファミリー 1 つ（一覧の 1 行）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemFamily {
    /// ファミリー名（英語の名前か、無ければ最初の名前）。
    pub name: String,
    /// 日本語のファミリー名（あれば）。
    pub name_ja: Option<String>,
    /// スタイル（太さ・斜体・スタイルの名前の順）。`SystemFonts::faces` の番号。
    pub faces: Vec<usize>,
}

/// OS に入っているフォントの一覧（描けるフォントだけ。輪郭の無いフォント・読めないフォントは入れない）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SystemFonts {
    faces: Vec<SystemFace>,
    families: Vec<SystemFamily>,
}

impl SystemFonts {
    /// OS のフォントのフォルダをなめて作る（遅い。画面のスレッドで呼ばない）。
    pub fn scan() -> SystemFonts {
        let mut db = fontdb::Database::new();
        db.load_system_fonts();
        Self::from_db(&db)
    }

    /// 決めたフォルダ（中も）のフォントだけで作る（試験・決まった環境のため）。
    pub fn from_dirs(dirs: &[&Path]) -> SystemFonts {
        let mut db = fontdb::Database::new();
        for dir in dirs {
            db.load_fonts_dir(dir);
        }
        Self::from_db(&db)
    }

    fn from_db(db: &fontdb::Database) -> SystemFonts {
        let mut faces = Vec::new();
        for info in db.faces() {
            let path = match &info.source {
                fontdb::Source::File(path) | fontdb::Source::SharedFile(path, _) => path.clone(),
                fontdb::Source::Binary(_) => continue,
            };
            let Some(Ok(description)) = db.with_face_data(info.id, text::describe_font) else {
                continue;
            };
            if description.names.family.is_empty() {
                continue;
            }
            faces.push(SystemFace {
                path,
                index: info.index,
                description,
            });
        }
        // 同じ道・番号は 1 つ（フォルダが重なって 2 度なめたとき）
        faces.sort_by(|a, b| (&a.path, a.index).cmp(&(&b.path, b.index)));
        faces.dedup_by(|a, b| a.path == b.path && a.index == b.index);
        faces.sort_by(|a, b| {
            let (x, y) = (&a.description, &b.description);
            x.names
                .family
                .to_lowercase()
                .cmp(&y.names.family.to_lowercase())
                .then(x.names.italic.cmp(&y.names.italic))
                .then(x.names.weight.cmp(&y.names.weight))
                .then(x.style.cmp(&y.style))
                .then(a.path.cmp(&b.path))
                .then(a.index.cmp(&b.index))
        });
        let mut families: Vec<SystemFamily> = Vec::new();
        for (i, face) in faces.iter().enumerate() {
            let d = &face.description;
            match families.last_mut() {
                Some(f) if f.name.to_lowercase() == d.names.family.to_lowercase() => {
                    // 同じスタイルの名前（同じフォントが別のフォルダにも入っている）は一覧に 1 つ
                    if !f
                        .faces
                        .iter()
                        .any(|&j| faces[j].description.names == d.names)
                    {
                        f.faces.push(i);
                    }
                    if f.name_ja.is_none() {
                        f.name_ja = d.family_ja.clone();
                    }
                }
                _ => families.push(SystemFamily {
                    name: d.names.family.clone(),
                    name_ja: d.family_ja.clone(),
                    faces: vec![i],
                }),
            }
        }
        SystemFonts { faces, families }
    }

    pub fn faces(&self) -> &[SystemFace] {
        &self.faces
    }

    /// ファミリーの一覧（英語の名前の順）。
    pub fn families(&self) -> &[SystemFamily] {
        &self.families
    }

    pub fn is_empty(&self) -> bool {
        self.faces.is_empty()
    }

    /// PostScript 名で探す（大文字・小文字を区別しない）。
    pub fn by_postscript(&self, name: &str) -> Option<&SystemFace> {
        if name.is_empty() {
            return None;
        }
        self.faces
            .iter()
            .find(|f| f.description.names.postscript.eq_ignore_ascii_case(name))
    }

    /// ファミリー名（英語・日本語の名前、大文字・小文字を区別しない）と太さと斜体かで探す。同じ太さの斜体か・斜体でないかが無ければ、
    /// 同じ太さのどちらか。
    pub fn by_family(&self, family: &str, weight: u16, italic: bool) -> Option<&SystemFace> {
        if family.is_empty() {
            return None;
        }
        let lower = family.to_lowercase();
        let family = self
            .families
            .iter()
            .find(|f| f.name.to_lowercase() == lower || f.name_ja.as_deref() == Some(family))?;
        let same_weight = || {
            family
                .faces
                .iter()
                .map(|&i| &self.faces[i])
                .filter(|f| f.description.names.weight == weight)
        };
        same_weight()
            .find(|f| f.description.names.italic == italic)
            .or_else(|| same_weight().next())
    }

    /// 名前で探す（CLI の `font`）: PostScript 名、無ければファミリー名の太さ 400 に近い斜体でないスタイル。
    pub fn by_name(&self, name: &str) -> Option<&SystemFace> {
        if let Some(face) = self.by_postscript(name) {
            return Some(face);
        }
        let lower = name.to_lowercase();
        let family = self
            .families
            .iter()
            .find(|f| f.name.to_lowercase() == lower || f.name_ja.as_deref() == Some(name))?;
        family
            .faces
            .iter()
            .map(|&i| &self.faces[i])
            .min_by_key(|f| {
                let n = &f.description.names;
                (n.italic, n.weight.abs_diff(400))
            })
    }
}

static SYSTEM: OnceLock<Arc<SystemFonts>> = OnceLock::new();

/// プロセスに 1 つの OS のフォントの一覧（初めての呼び出しでなめる。遅いので画面のスレッドでは [`loaded`] を見る）。
pub fn system() -> Arc<SystemFonts> {
    SYSTEM.get_or_init(|| Arc::new(SystemFonts::scan())).clone()
}

/// もうできている OS のフォントの一覧（まだなら None。待たない）。
pub fn loaded() -> Option<Arc<SystemFonts>> {
    SYSTEM.get().cloned()
}

/// フォントのファイルを読む（大きさの上限つき）。
pub fn read_font_file(path: &Path) -> std::io::Result<Arc<[u8]>> {
    let len = std::fs::metadata(path)?.len();
    if len > MAX_FONT_FILE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "font file too large",
        ));
    }
    Ok(Arc::from(std::fs::read(path)?))
}

/// OS のフォントを読み、テキストレイヤーの値にする（道・番号・SHA-256・名前）。読めなければ None。
pub fn load_face(face: &SystemFace) -> Option<(TextFont, Arc<[u8]>)> {
    let bytes = read_font_file(&face.path).ok()?;
    text::check_font(&bytes, face.index).ok()?;
    let font = text::file_font(&face.path.to_string_lossy(), face.index, &bytes);
    Some((font, bytes))
}

/// テキストレイヤーのフォントを探した結果。
#[derive(Clone)]
pub enum Lookup {
    /// 文書の値と同じ中身（SHA-256 が同じ）のフォント。`font` は見つけた所（道・番号）に直した値。
    Same { font: TextFont, bytes: Arc<[u8]> },
    /// 名前・道で見つけたが、中身が文書の値と違う。描き直すと `font` の値（見つけたフォント）になる。
    Different { font: TextFont, bytes: Arc<[u8]> },
    /// どこにも無い（描いた画素のまま。値の編集は断る）。
    Missing,
}

impl std::fmt::Debug for Lookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Lookup::Same { font, .. } => f.debug_tuple("Same").field(font).finish(),
            Lookup::Different { font, .. } => f.debug_tuple("Different").field(font).finish(),
            Lookup::Missing => f.write_str("Missing"),
        }
    }
}

/// 覚えた道が、場所として使える絶対の道か。ファイル名だけ・相対の道（配布用の写しは、フォントの場所をファイル名だけにする）は、今の作業フォルダー
/// によって別のファイルを指すので、場所としては使わず、名前（PostScript 名・ファミリー名）で探す。
fn usable_path(path: &str) -> bool {
    Path::new(path).is_absolute()
}

/// 覚えた道のファイルが、文書の値と同じ中身か（OS の一覧を使わない速い道）。同じなら [`Lookup::Same`]。
pub fn find_at_path(font: &TextFont) -> Option<Lookup> {
    let TextFont::File { path, index, .. } = font else {
        return None;
    };
    if !usable_path(path) {
        return None;
    }
    let bytes = read_font_file(Path::new(path)).ok()?;
    (text::font_matches(font, &bytes) && text::check_font(&bytes, *index).is_ok()).then(|| {
        Lookup::Same {
            font: font.clone(),
            bytes,
        }
    })
}

/// テキストレイヤーのファイルのフォントを探す（同梱のフォントは呼び手が探す。渡されたら [`Lookup::Missing`]）。順は頭の説明。
pub fn find(font: &TextFont, system: &SystemFonts) -> Lookup {
    let TextFont::File { path, names, .. } = font else {
        return Lookup::Missing;
    };
    if let Some(same) = find_at_path(font) {
        return same;
    }
    let candidates = [
        system.by_postscript(&names.postscript),
        system.by_family(&names.family, names.weight, names.italic),
    ];
    for face in candidates.into_iter().flatten() {
        if let Some(found) = load_face(face) {
            return judged(font, found);
        }
    }
    if let Some(bytes) = usable_path(path)
        .then(|| read_font_file(Path::new(path)).ok())
        .flatten()
    {
        let index = font.index();
        if text::check_font(&bytes, index).is_ok() {
            let found = text::file_font(path, index, &bytes);
            return judged(font, (found, bytes));
        }
    }
    Lookup::Missing
}

fn judged(wanted: &TextFont, (font, bytes): (TextFont, Arc<[u8]>)) -> Lookup {
    if text::font_matches(wanted, &bytes) {
        Lookup::Same { font, bytes }
    } else {
        Lookup::Different { font, bytes }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yolu-fonts-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let fonts = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yolu-app/assets/fonts");
        std::fs::copy(
            fonts.join("BIZUDPGothic-Regular.ttf"),
            dir.join("Regular.ttf"),
        )
        .unwrap();
        std::fs::copy(
            fonts.join("BIZUDPGothic-Bold.ttf"),
            dir.join("sub/Bold.ttf"),
        )
        .unwrap();
        // フォントでないファイルは入れない
        std::fs::write(dir.join("broken.ttf"), b"not a font").unwrap();
        dir
    }

    #[test]
    fn a_folder_is_listed_by_family_with_styles_by_weight() {
        let dir = fixture_dir("list");
        let list = SystemFonts::from_dirs(&[&dir]);
        assert_eq!(list.faces().len(), 2, "{:?}", list.faces());
        assert_eq!(list.families().len(), 1);
        let family = &list.families()[0];
        assert_eq!(family.name, "BIZ UDPGothic");
        assert_eq!(family.name_ja.as_deref(), Some("BIZ UDPゴシック"));
        let styles: Vec<String> = family
            .faces
            .iter()
            .map(|&i| list.faces()[i].style())
            .collect();
        assert_eq!(styles, ["Regular", "Bold"]);
        let bold = list
            .by_postscript("bizudpgothic-bold")
            .expect("大文字・小文字を問わない");
        assert_eq!(bold.description.names.weight, 700);
        assert_eq!(
            list.by_family("BIZ UDPゴシック", 700, false),
            Some(bold),
            "日本語の名前でも探せる"
        );
        assert_eq!(
            list.by_name("BIZ UDPGothic")
                .map(|f| f.description.names.weight),
            Some(400)
        );
        assert!(list.by_family("BIZ UDPGothic", 900, false).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 配布用の写しはフォントの場所をファイル名だけにする: 場所としては使わず、名前（PostScript 名・ファミリー名）で OS のフォントから探して見つける。
    /// 今の作業フォルダーから相対に辿れる同じ名前のファイルは拾わない（作業フォルダーによって別のファイルを指す）。
    #[test]
    fn a_font_with_only_a_file_name_is_found_by_name_and_never_from_the_working_folder() {
        let dir = fixture_dir("name-only");
        let list = SystemFonts::from_dirs(&[&dir]);
        let regular = dir.join("Regular.ttf");
        let bytes = std::fs::read(&regular).unwrap();
        let absolute = text::file_font(&regular.to_string_lossy(), 0, &bytes);
        let TextFont::File {
            index,
            sha256,
            names,
            ..
        } = &absolute
        else {
            unreachable!()
        };
        let with_path = |path: &str| TextFont::File {
            path: path.into(),
            index: *index,
            sha256: *sha256,
            names: names.clone(),
        };
        // ファイル名だけ: 場所は使えないが、OS のフォントを名前で探して同じ中身を見つける（見つけた場所の絶対の道の値になる）
        let Lookup::Same { font, .. } = find(&with_path("Regular.ttf"), &list) else {
            panic!("名前で見つからない")
        };
        assert_eq!(font, absolute);
        assert!(find_at_path(&with_path("Regular.ttf")).is_none());
        // 作業フォルダーから相対に辿れるフォントのファイルがあっても、名前の無い（OS に無い）ときは拾わない
        let relative = "../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf";
        assert!(
            Path::new(relative).is_file(),
            "試験は yolu-io のフォルダーで動く"
        );
        let nothing = SystemFonts::from_dirs(&[]);
        assert!(matches!(
            find(&with_path(relative), &nothing),
            Lookup::Missing
        ));
        assert!(find_at_path(&with_path(relative)).is_none());
        // 絶対の道なら、今までどおり場所から読める
        let absolute_path = Path::new(relative).canonicalize().unwrap();
        assert!(matches!(
            find(&with_path(&absolute_path.to_string_lossy()), &nothing),
            Lookup::Different { .. } | Lookup::Same { .. }
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_moved_font_is_found_by_name_and_a_changed_one_is_different() {
        let dir = fixture_dir("find");
        let list = SystemFonts::from_dirs(&[&dir]);
        let regular = dir.join("Regular.ttf");
        let bytes = std::fs::read(&regular).unwrap();
        let font = text::file_font(&regular.to_string_lossy(), 0, &bytes);
        assert!(matches!(find(&font, &list), Lookup::Same { .. }));
        // 道が変わっても PostScript 名で同じ中身を見つける（値は見つけた道）
        let moved = match &font {
            TextFont::File {
                index,
                sha256,
                names,
                ..
            } => TextFont::File {
                path: dir.join("gone.ttf").to_string_lossy().into_owned(),
                index: *index,
                sha256: *sha256,
                names: names.clone(),
            },
            _ => unreachable!(),
        };
        let Lookup::Same { font: found, .. } = find(&moved, &list) else {
            panic!("名前で見つからない")
        };
        assert_eq!(found, font);
        // 中身の違う同じ名前（版の違うフォント）は Different
        let other = match &moved {
            TextFont::File {
                path, index, names, ..
            } => TextFont::File {
                path: path.clone(),
                index: *index,
                sha256: [7; 32],
                names: names.clone(),
            },
            _ => unreachable!(),
        };
        assert!(matches!(find(&other, &list), Lookup::Different { font: f, .. } if f == font));
        // 名前も道も無い
        let missing = match &other {
            TextFont::File { index, sha256, .. } => TextFont::File {
                path: dir.join("gone.ttf").to_string_lossy().into_owned(),
                index: *index,
                sha256: *sha256,
                names: text::FontNames {
                    family: "Nothing Like It".into(),
                    postscript: "NothingLikeIt-Regular".into(),
                    ..Default::default()
                },
            },
            _ => unreachable!(),
        };
        assert!(matches!(find(&missing, &list), Lookup::Missing));
        assert!(matches!(
            find(&TextFont::Bundled(text::DEFAULT_FONT.into()), &list),
            Lookup::Missing
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
