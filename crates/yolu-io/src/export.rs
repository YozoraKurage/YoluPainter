//! 書き出しのテンプレート（`yolu_core::export`）の画像を、フォルダへ PNG として書く。
//!
//! 名前は Unity 版と同じ規則: `<名前>[_<セット名>]_<接尾辞>.png`（セットが複数のときだけ `_<セット名>`）。書くのは 3 段:
//! (1) 名前・上書き・大きさを確かめる（ここで断れば何も作らない）、(2) 画像を 1 枚ずつ作り、PNG にして**隠しの一時ファイル**へ書き、
//! 読み戻して RGBA が一致することを確かめる、(3) 全部が済んでから、1 枚ずつ置き換える（一時ファイルからの `rename`。既にある
//! ファイルを置き換えない設定のときは `hard_link` で、先にできていたら断る）。(2) までに失敗・取消すれば、元のファイルは 1 つも
//! 変わらず一時ファイルも残らない。(3) の途中で失敗したときだけ、済んだ分が置き換わっているので [`ExportError::Partial`] で知らせる
//! （新しく作った分は消し、置き換えた分は元に戻せない）。
//!
//! Unity のインポートの設定（sRGB・ノーマルマップ）はファイルに書かない。返す [`WrittenImage`] に載せるので、Live Link や画面が使う。

use std::collections::HashSet;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use yolu_core::export::{
    build, should_write, ExportError as CalcError, ExportImage, ExportImageKind, ExportTemplate,
};
use yolu_core::padding::{self, Reach};
use yolu_core::Document;

/// PNG の 1 辺の上限（Unity 版の `ImageContent.MaxSide` と同じ）。
pub const MAX_SIDE: u32 = 8192;
/// ファイル名の長さの上限（UTF-8 のバイト数。多くのファイルシステムの上限）。
const MAX_NAME_BYTES: usize = 255;

static NEXT: AtomicU64 = AtomicU64::new(0);

/// 書き出しの失敗。
#[derive(Debug)]
pub enum ExportError {
    /// ファイル名にできない名前（空・使えない文字・長すぎる）。
    InvalidName(String),
    /// 同じファイル名（大文字小文字を区別しない）になる画像が複数。
    NameClash(Vec<String>),
    /// 置き換えない設定（既定）で、もうあるファイル。何も書いていない。
    WouldReplace(Vec<PathBuf>),
    /// 行き先がフォルダ・リンクなど、通常のファイルでない。
    NotAFile(PathBuf),
    /// 取り消した。何も置き換えていない。
    Cancelled,
    /// 画像の大きさ・長さが合わない、PNG の上限（[`MAX_SIDE`]）を超える。
    InvalidImage(String),
    /// 画像を作る計算（作業の予算超過・AO の長さなど）か、パディングが断った。
    Calc(CalcError),
    /// ファイルの読み書きの失敗。
    Io(String),
    /// 一時ファイルの読み戻しが、書いた画像と一致しなかった。
    Verify(String),
    /// 置き換えの途中で失敗した。`written` はもう置き換わった分。
    Partial {
        written: Vec<WrittenImage>,
        cause: String,
    },
}

impl fmt::Display for ExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExportError::InvalidName(n) => write!(f, "ファイル名にできない名前: {n}"),
            ExportError::NameClash(names) => {
                write!(f, "同じファイルになる画像がある: {}", names.join("、"))
            }
            ExportError::WouldReplace(paths) => write!(
                f,
                "もうあるファイルは置き換えない: {}",
                paths
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            ExportError::NotAFile(p) => write!(f, "通常のファイルではない: {}", p.display()),
            ExportError::Cancelled => write!(f, "取り消した（何も置き換えていない）"),
            ExportError::InvalidImage(why) => write!(f, "画像が書けない: {why}"),
            ExportError::Calc(e) => write!(f, "{e}"),
            ExportError::Io(why) => write!(f, "ファイルの操作に失敗: {why}"),
            ExportError::Verify(why) => write!(f, "書いた PNG の読み戻しが一致しない: {why}"),
            ExportError::Partial { written, cause } => write!(
                f,
                "置き換えの途中で失敗した（{} 枚は置き換わった）: {cause}",
                written.len()
            ),
        }
    }
}

impl std::error::Error for ExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ExportError::Calc(e) => Some(e),
            _ => None,
        }
    }
}

impl From<CalcError> for ExportError {
    /// 計算が取り消しで止まったときは、書き出しの取消（[`ExportError::Cancelled`]）として返す（`Calc` に包まない）。
    fn from(e: CalcError) -> Self {
        match e {
            CalcError::Cancelled => ExportError::Cancelled,
            other => ExportError::Calc(other),
        }
    }
}

impl From<std::io::Error> for ExportError {
    fn from(e: std::io::Error) -> Self {
        ExportError::Io(e.to_string())
    }
}

impl From<ExportError> for crate::Error {
    fn from(e: ExportError) -> Self {
        crate::Error::InvalidData(e.to_string())
    }
}

/// もうあるファイルの扱い。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Overwrite {
    /// 1 つでもあれば、何も書かずに [`ExportError::WouldReplace`]（既定。呼び手が [`existing_files`] で見せて確かめてから `Replace`）。
    #[default]
    Refuse,
    /// 通常のファイルなら置き換える。
    Replace,
}

/// 書き出しの設定。
#[derive(Clone, Copy, Debug, Default)]
pub struct WriteOptions<'a> {
    pub overwrite: Overwrite,
    /// 立つと、次の画像の前か塗り広げの途中で [`ExportError::Cancelled`]（置き換えを始めた後は止めない）。
    pub cancel: Option<&'a AtomicBool>,
}

/// 書く画像 1 枚の取り決め（中身は [`write_images`] が呼び手の関数から 1 枚ずつ受け取る）。
#[derive(Clone, Debug)]
pub struct ExportFile<'a> {
    /// ファイル名（[`file_name`] の結果）。
    pub name: String,
    pub image: &'a ExportImage,
    pub width: u32,
    pub height: u32,
}

/// 書いた画像 1 枚。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WrittenImage {
    pub path: PathBuf,
    pub file_name: String,
    pub suffix: String,
    pub kind: ExportImageKind,
    /// Unity の取り込み: sRGB の色か（そうでなければリニア）。
    pub srgb: bool,
    /// Unity の取り込み: ノーマルマップか。
    pub normal_map: bool,
    /// Unity の取り込み: アルファを透明度として扱うか。
    pub alpha_is_transparency: bool,
    pub width: u32,
    pub height: u32,
    /// 既にあったファイルを置き換えたか（false なら新しく作った）。
    pub replaced: bool,
}

// ───────── 名前 ─────────

/// ファイル名に使えない文字（Windows の分を全部。Unity 版は実行する OS の `Path.GetInvalidFileNameChars()` だが、どの OS で書いても
/// 同じ名前になるように揃える）を `_` に置き換え、前後の空白を除く。
pub fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_control() || matches!(c, '"' | '*' | '/' | ':' | '<' | '>' | '?' | '\\' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// 画像のファイル名: `<名前>[_<セット名>]_<接尾辞>.png`。`set_name` はセットが複数のときだけ渡す（Unity 版の `SetFileSuffix`）。
/// 名前とセット名は [`sanitize`] し、接尾辞は使えない文字・空を断る。
pub fn file_name(
    stem: &str,
    set_name: Option<&str>,
    image: &ExportImage,
) -> Result<String, ExportError> {
    let stem_clean = sanitize(stem);
    if stem_clean.is_empty() {
        return Err(ExportError::InvalidName(stem.to_string()));
    }
    let suffix = image.suffix();
    if suffix.is_empty() || sanitize(suffix) != suffix || suffix.starts_with('.') {
        return Err(ExportError::InvalidName(suffix.to_string()));
    }
    let set_part = set_name
        .map(|s| format!("_{}", sanitize(s)))
        .unwrap_or_default();
    let name = format!("{stem_clean}{set_part}_{suffix}.png");
    if name.len() > MAX_NAME_BYTES {
        return Err(ExportError::InvalidName(name));
    }
    Ok(name)
}

/// 計画した 1 画像。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlannedFile {
    /// セットの番号（渡した並びの）。
    pub set: usize,
    /// テンプレートの画像の番号。
    pub image: usize,
    pub file_name: String,
}

/// 計画に渡す 1 セット。
#[derive(Clone, Copy)]
pub struct PlanSet<'a> {
    /// セットが複数のときだけ Some（ファイル名に入る）。
    pub name: Option<&'a str>,
    pub document: &'a Document,
    /// 焼いた AO があるか（AO を読む画像を書くかに効く）。
    pub has_occlusion: bool,
}

/// 全セットの、書く画像（読むものがある画像だけ。[`should_write`]）とそのファイル名。
pub fn plan_template(
    stem: &str,
    sets: &[PlanSet<'_>],
    template: &ExportTemplate,
) -> Result<Vec<PlannedFile>, ExportError> {
    let mut planned = Vec::new();
    for (s, set) in sets.iter().enumerate() {
        for (i, image) in template.images.iter().enumerate() {
            if should_write(set.document, image, set.has_occlusion) {
                planned.push(PlannedFile {
                    set: s,
                    image: i,
                    file_name: file_name(stem, set.name, image)?,
                });
            }
        }
    }
    Ok(planned)
}

/// 同じファイル名（大文字小文字を区別しない）になる名前（Unity 版の重なりの確かめと同じ）。
pub fn clashes<'a>(names: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut clashing: Vec<String> = Vec::new();
    for name in names {
        let key = name.to_lowercase();
        if !seen.insert(key) && !clashing.iter().any(|c| c.eq_ignore_ascii_case(name)) {
            clashing.push(name.to_string());
        }
    }
    clashing
}

/// `dir` にもうある（置き換えることになる）ファイル。フォルダやリンクも含む（[`write_images`] はそれを置き換えない）。
pub fn existing_files<'a>(dir: &Path, names: impl IntoIterator<Item = &'a str>) -> Vec<PathBuf> {
    names
        .into_iter()
        .map(|n| dir.join(n))
        .filter(|p| fs::symlink_metadata(p).is_ok())
        .collect()
}

// ───────── 書く ─────────

/// 画像の中身（straight RGBA8、行は下から上）を `produce(i)` から 1 枚ずつ受け取り、`files[i]` の名前で `dir` へ PNG として書く
/// （手順はモジュールの説明）。中身は 1 枚ずつしか持たない。
pub fn write_images(
    dir: &Path,
    files: &[ExportFile<'_>],
    options: &WriteOptions<'_>,
    mut produce: impl FnMut(usize) -> Result<Vec<u8>, ExportError>,
) -> Result<Vec<WrittenImage>, ExportError> {
    write_images_inner(dir, files, options, &mut produce, &mut |_, _| Ok(()))
}

/// 試験が各段で失敗を差し込む口（`phase(段, 一時ファイルか行き先)`）。
type Phase<'a> = &'a mut dyn FnMut(&str, &Path) -> Result<(), ExportError>;

/// 一時ファイル。置き換えるまでの間、持ち主が消えれば（失敗・取消・パニック）消す。
struct Pending {
    temp: PathBuf,
    target: PathBuf,
    committed: bool,
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

fn io(what: &str, path: &Path, e: std::io::Error) -> ExportError {
    ExportError::Io(format!("{what}（{}）: {e}", path.display()))
}

fn cancelled(options: &WriteOptions<'_>) -> bool {
    options.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

fn write_images_inner(
    dir: &Path,
    files: &[ExportFile<'_>],
    options: &WriteOptions<'_>,
    produce: &mut dyn FnMut(usize) -> Result<Vec<u8>, ExportError>,
    phase: Phase<'_>,
) -> Result<Vec<WrittenImage>, ExportError> {
    // (1) 作る前に断れることは全部ここで
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    for f in files {
        if f.name.is_empty()
            || f.name == "."
            || f.name == ".."
            || sanitize(&f.name) != f.name
            || f.name.len() > MAX_NAME_BYTES
        {
            return Err(ExportError::InvalidName(f.name.clone()));
        }
        if f.width == 0 || f.height == 0 || f.width > MAX_SIDE || f.height > MAX_SIDE {
            return Err(ExportError::InvalidImage(format!(
                "{}: 大きさは 1〜{MAX_SIDE}（{} × {}）",
                f.name, f.width, f.height
            )));
        }
    }
    let clash = clashes(names.iter().copied());
    if !clash.is_empty() {
        return Err(ExportError::NameClash(clash));
    }
    let mut existing = Vec::new();
    for f in files {
        let path = dir.join(&f.name);
        match fs::symlink_metadata(&path) {
            Ok(m) if m.is_file() => existing.push(path),
            Ok(_) => return Err(ExportError::NotAFile(path)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io("行き先を調べられない", &path, e)),
        }
    }
    if options.overwrite == Overwrite::Refuse && !existing.is_empty() {
        return Err(ExportError::WouldReplace(existing));
    }
    if cancelled(options) {
        return Err(ExportError::Cancelled);
    }

    // (2) 1 枚ずつ作り、一時ファイルへ書き、読み戻して確かめる
    let mut pending: Vec<Pending> = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        if cancelled(options) {
            return Err(ExportError::Cancelled);
        }
        let rgba = produce(i)?;
        let expected = f.width as usize * f.height as usize * 4;
        if rgba.len() != expected {
            return Err(ExportError::InvalidImage(format!(
                "{}: 中身は幅 × 高さ × 4 バイト（{} バイトのはずが {}）",
                f.name,
                expected,
                rgba.len()
            )));
        }
        let png = crate::composite_png::encode_export(&rgba, f.width, f.height)
            .map_err(|e| ExportError::InvalidImage(format!("{}: {e}", f.name)))?;
        fs::create_dir_all(dir).map_err(|e| io("フォルダを作れない", dir, e))?;
        let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
        // 行き先の名前から切り離す（名前が上限いっぱいでも一時ファイルを作れるように）
        let temp = dir.join(format!(
            ".yolu-export-{}-{nonce}.pending~",
            std::process::id()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| io("一時ファイルを作れない", &temp, e))?;
        // 作れたからは自分のもの。以降の失敗・取消・パニックでは、持ち主（guard）が消える
        let guard = Pending {
            temp: temp.clone(),
            target: dir.join(&f.name),
            committed: false,
        };
        file.write_all(&png).map_err(|e| io("書けない", &temp, e))?;
        file.sync_all()
            .map_err(|e| io("書き出しを確定できない", &temp, e))?;
        drop(file);
        drop(png);
        phase("flushed", &temp)?;
        verify_png(&temp, &rgba, f.width, f.height)?;
        phase("verified", &temp)?;
        pending.push(guard);
    }
    if cancelled(options) {
        return Err(ExportError::Cancelled);
    }
    phase("before-commit", dir)?;

    // (3) 置き換える。ここから先は止めない
    let mut written: Vec<WrittenImage> = Vec::with_capacity(files.len());
    let mut failure: Option<ExportError> = None;
    for (f, p) in files.iter().zip(pending.iter_mut()) {
        match phase("before-rename", &p.target)
            .and_then(|()| commit(p, options.overwrite, &mut *phase))
        {
            Ok(replaced) => written.push(WrittenImage {
                path: p.target.clone(),
                file_name: f.name.clone(),
                suffix: f.image.suffix().to_string(),
                kind: f.image.kind(),
                srgb: f.image.srgb(),
                normal_map: f.image.normal_map(),
                alpha_is_transparency: f.image.alpha_is_transparency(),
                width: f.width,
                height: f.height,
                replaced,
            }),
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    let Some(cause) = failure else {
        return Ok(written);
    };
    // 新しく作った分は消す（元からあったファイルではない）。置き換えた分は戻せないので知らせる
    let mut kept = Vec::new();
    for w in written {
        if w.replaced {
            kept.push(w);
        } else {
            let _ = fs::remove_file(&w.path);
        }
    }
    if kept.is_empty() {
        Err(cause)
    } else {
        Err(ExportError::Partial {
            written: kept,
            cause: cause.to_string(),
        })
    }
}

/// 一時ファイルを行き先へ。置き換えた（元からあった）なら true。
fn commit(p: &mut Pending, overwrite: Overwrite, phase: Phase<'_>) -> Result<bool, ExportError> {
    let target = p.target.clone();
    let existed = match fs::symlink_metadata(&target) {
        Ok(m) if m.is_file() => true,
        Ok(_) => return Err(ExportError::NotAFile(target)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(io("行き先を調べられない", &target, e)),
    };
    match overwrite {
        Overwrite::Replace => {
            fs::rename(&p.temp, &target).map_err(|e| io("置き換えられない", &target, e))?;
            p.committed = true;
            Ok(existed)
        }
        Overwrite::Refuse => {
            if existed {
                return Err(ExportError::WouldReplace(vec![target]));
            }
            // 先にできていたら断る（調べてから置くまでの間に作られても上書きしない）
            phase("before-link", &target)?;
            match fs::hard_link(&p.temp, &target) {
                Ok(()) => {
                    let _ = fs::remove_file(&p.temp);
                    p.committed = true;
                    Ok(false)
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    Err(ExportError::WouldReplace(vec![target]))
                }
                Err(_) => {
                    // ハードリンクを作れないファイルシステム: 調べ直してから置く（間に作られる隙は残る）
                    if fs::symlink_metadata(&target).is_ok() {
                        return Err(ExportError::WouldReplace(vec![target]));
                    }
                    fs::rename(&p.temp, &target).map_err(|e| io("置けない", &target, e))?;
                    p.committed = true;
                    Ok(false)
                }
            }
        }
    }
}

/// 書いた PNG を読み戻し、1 行ずつ画像と照らす（CRC・zlib の検査も通る。ファイルは上の行から、画像は下の行から）。
fn verify_png(path: &Path, rgba: &[u8], width: u32, height: u32) -> Result<(), ExportError> {
    let file = File::open(path).map_err(|e| io("読み戻せない", path, e))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_limits(png::Limits {
        bytes: crate::MAX_ENTRY_BYTES,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|e| ExportError::Verify(format!("ヘッダー: {e}")))?;
    {
        let info = reader.info();
        if info.width != width
            || info.height != height
            || info.color_type != png::ColorType::Rgba
            || info.bit_depth != png::BitDepth::Eight
            || info.interlaced
        {
            return Err(ExportError::Verify("ヘッダーが違う".into()));
        }
    }
    let stride = width as usize * 4;
    for top in 0..height as usize {
        let row = reader
            .next_row()
            .map_err(|e| ExportError::Verify(format!("{top} 行目: {e}")))?
            .ok_or_else(|| ExportError::Verify(format!("{top} 行目が無い")))?;
        let source = &rgba[(height as usize - 1 - top) * stride..][..stride];
        if row.data() != source {
            return Err(ExportError::Verify(format!("{top} 行目の画素が違う")));
        }
    }
    reader
        .finish()
        .map_err(|e| ExportError::Verify(format!("終端: {e}")))?;
    Ok(())
}

// ───────── 1 セットぶん ─────────

/// 書き出しの塗り広げ（UV の外へ。`yolu_core::padding`）。
#[derive(Clone, Copy, Debug)]
pub struct PaddingSpec<'a> {
    /// UV の三角形が覆うテクセルの印（[`padding::coverage`]。文書と同じ大きさ）。
    pub coverage: &'a [bool],
    pub reach: Reach,
}

/// 1 セットぶんの書き出しの入力。
#[derive(Clone, Copy)]
pub struct SetExport<'a> {
    /// 名前の元（プロジェクトのファイル名の拡張子なしなど。無ければ `Texture`）。
    pub stem: &'a str,
    /// セットが複数のときだけ Some。
    pub set_name: Option<&'a str>,
    pub document: &'a Document,
    /// 焼いた AO（1 テクセル 1 バイト。無いテクセルは 255）。無ければ None。
    pub occlusion: Option<&'a [u8]>,
    /// None なら塗り広げない。
    pub padding: Option<PaddingSpec<'a>>,
    /// 画像を作る作業と塗り広げの作業のメモリの上限（Unity 版は `PainterSettings.StrokeBudgetBytes`）。
    pub max_working_bytes: u64,
}

/// 塗り広げがどうなったか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaddingOutcome {
    /// 頼まれなかった。
    NotRequested,
    /// 覆いが空（UV の三角形が 1 つも無い）なので掛けなかった。
    NoCoverage,
    /// 掛けた。
    Applied,
}

/// 1 セットぶんの書き出しの結果。
#[derive(Clone, Debug)]
pub struct TemplateReport {
    pub written: Vec<WrittenImage>,
    /// 読むものが無くて書かなかった画像の接尾辞。
    pub skipped: Vec<String>,
    pub padding: PaddingOutcome,
}

/// 1 セットの、テンプレートの画像のうち読むものがある画像を作って（塗り広げて）書く。何も書く画像が無ければフォルダも作らず空を返す。
pub fn write_template(
    dir: &Path,
    template: &ExportTemplate,
    set: &SetExport<'_>,
    options: &WriteOptions<'_>,
) -> Result<TemplateReport, ExportError> {
    write_template_inner(dir, template, set, options, &mut |_| {})
}

/// [`write_template`]。`after_build(i)` は i 枚目を作り終えて、塗り広げる前に呼ばれる（試験が取消の旗をそこで立てる口）。
fn write_template_inner(
    dir: &Path,
    template: &ExportTemplate,
    set: &SetExport<'_>,
    options: &WriteOptions<'_>,
    after_build: &mut dyn FnMut(usize),
) -> Result<TemplateReport, ExportError> {
    let doc = set.document;
    let has_occlusion = set.occlusion.is_some();
    let mut files = Vec::new();
    let mut images = Vec::new();
    let mut skipped = Vec::new();
    for image in &template.images {
        if should_write(doc, image, has_occlusion) {
            files.push(ExportFile {
                name: file_name(set.stem, set.set_name, image)?,
                image,
                width: doc.width(),
                height: doc.height(),
            });
            images.push(image);
        } else {
            skipped.push(image.suffix().to_string());
        }
    }
    let padding = match set.padding {
        None => PaddingOutcome::NotRequested,
        Some(p) if !p.coverage.contains(&true) => PaddingOutcome::NoCoverage,
        Some(_) => PaddingOutcome::Applied,
    };
    if let Some(p) = set.padding {
        if p.coverage.len() as u64 != doc.width() as u64 * doc.height() as u64 {
            return Err(ExportError::Calc(CalcError::InvalidArgument(
                "覆いはキャンバスと同じ大きさ（幅 × 高さ）",
            )));
        }
    }
    let written = write_images(dir, &files, options, |i| {
        let pixels = build(doc, images[i], set.occlusion, set.max_working_bytes)?;
        after_build(i);
        match set.padding {
            Some(p) if padding == PaddingOutcome::Applied => Ok(padding::dilate_cancellable(
                &pixels,
                doc.width(),
                doc.height(),
                p.coverage,
                p.reach,
                set.max_working_bytes,
                options.cancel,
            )?),
            _ => Ok(pixels),
        }
    })?;
    Ok(TemplateReport {
        written,
        skipped,
        padding,
    })
}

#[cfg(test)]
#[allow(clippy::chunks_exact_to_as_chunks)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use yolu_core::export::ExportScalar;
    use yolu_core::{Channel, TileCoord};

    // ───────── 名前 ─────────

    #[test]
    fn sanitize_replaces_unsafe_characters_and_trims() {
        assert_eq!(
            sanitize("  a/b\\c:d*e?f\"g<h>i|j\u{7}k  "),
            "a_b_c_d_e_f_g_h_i_j_k"
        );
        assert_eq!(sanitize("キャラ 衣装"), "キャラ 衣装");
        assert_eq!(sanitize("../x"), ".._x");
    }

    #[test]
    fn names_follow_the_unity_rule() {
        let albedo = ExportImage::of("Albedo", ExportImageKind::BaseColor);
        assert_eq!(file_name("Tex", None, &albedo).unwrap(), "Tex_Albedo.png");
        assert_eq!(
            file_name("Tex", Some("Body"), &albedo).unwrap(),
            "Tex_Body_Albedo.png"
        );
        assert_eq!(
            file_name("Tex", Some("a/b"), &albedo).unwrap(),
            "Tex_a_b_Albedo.png"
        );
        assert_eq!(
            file_name("Tex", Some(""), &albedo).unwrap(),
            "Tex__Albedo.png"
        );
        assert!(file_name("  ", None, &albedo).is_err());
        let bad = ExportImage::pack(
            "A/B",
            ExportScalar::Zero,
            ExportScalar::Zero,
            ExportScalar::Zero,
            ExportScalar::One,
        );
        assert!(file_name("Tex", None, &bad).is_err());
        let hidden = ExportImage::of(".x", ExportImageKind::Normal);
        assert!(file_name("Tex", None, &hidden).is_err());
        assert!(file_name(&"あ".repeat(100), None, &albedo).is_err()); // 300 バイトを超える
    }

    #[test]
    fn clashes_ignore_case() {
        assert_eq!(clashes(["a.png", "B.png", "b.PNG"]), vec!["b.PNG"]);
        assert!(clashes(["a.png", "b.png"]).is_empty());
    }

    // ───────── 書く ─────────

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let p = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/io-export-tests")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn out(&self) -> PathBuf {
            self.0.join("out")
        }
        /// フォルダの中のファイル名（並べた）。
        fn names(&self) -> Vec<String> {
            let Ok(entries) = fs::read_dir(self.out()) else {
                return Vec::new();
            };
            let mut names: Vec<String> = entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// 擬似乱数（試験の入力用）。
    fn lcg(state: &mut u64) -> u8 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*state >> 33) as u8
    }

    fn paint(doc: &mut Document, id: yolu_core::LayerId, channel: Channel, seed: u64) {
        let ts = doc.tile_size() as usize;
        let mut state = seed;
        doc.set_channel_enabled(id, channel, true).unwrap();
        for ty in 0..doc.height().div_ceil(ts as u32) {
            for tx in 0..doc.width().div_ceil(ts as u32) {
                let mut bytes: Vec<u8> = (0..ts * ts * 4).map(|_| lcg(&mut state)).collect();
                for (i, px) in bytes.chunks_exact_mut(4).enumerate() {
                    // キャンバスの外の余白は 0
                    if tx as usize * ts + i % ts >= doc.width() as usize
                        || ty as usize * ts + i / ts >= doc.height() as usize
                    {
                        px.fill(0);
                    }
                }
                doc.import_tile(id, channel, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }

    /// 16 × 12 の文書: Color・Metallic・Roughness・Emission を塗る（Height・Normal・AO は無い）。
    fn document() -> Document {
        let mut doc = Document::with_tile_size(16, 12, 8).unwrap();
        let id = doc.add_layer("a").unwrap();
        paint(&mut doc, id, Channel::Color, 1);
        paint(&mut doc, id, Channel::Metallic, 2);
        paint(&mut doc, id, Channel::Roughness, 3);
        paint(&mut doc, id, Channel::Emission, 4);
        doc.clear_history().unwrap();
        doc
    }

    fn set_export<'a>(doc: &'a Document) -> SetExport<'a> {
        SetExport {
            stem: "Tex",
            set_name: None,
            document: doc,
            occlusion: None,
            padding: None,
            max_working_bytes: u64::MAX,
        }
    }

    /// 書いた PNG を読み戻す（下の行から）。
    fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
        let decoder = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
        let mut reader = decoder.read_info().unwrap();
        let (w, h) = (reader.info().width, reader.info().height);
        let mut top_down = vec![0u8; w as usize * h as usize * 4];
        reader.next_frame(&mut top_down).unwrap();
        let stride = w as usize * 4;
        let mut bottom_up = Vec::with_capacity(top_down.len());
        for row in top_down.chunks_exact(stride).rev() {
            bottom_up.extend_from_slice(row);
        }
        (w, h, bottom_up)
    }

    fn standard() -> ExportTemplate {
        ExportTemplate::unity_standard()
    }

    #[test]
    fn written_pngs_read_back_to_the_built_images() {
        let s = Scratch::new();
        let doc = document();
        let template = standard();
        let report = write_template(
            &s.out(),
            &template,
            &set_export(&doc),
            &WriteOptions::default(),
        )
        .unwrap();
        let names: Vec<&str> = report
            .written
            .iter()
            .map(|w| w.file_name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "Tex_Albedo.png",
                "Tex_MetallicSmoothness.png",
                "Tex_Emission.png"
            ]
        );
        assert_eq!(report.skipped, ["Normal", "Height", "Occlusion"]);
        assert_eq!(report.padding, PaddingOutcome::NotRequested);
        assert_eq!(
            s.names(),
            [
                "Tex_Albedo.png",
                "Tex_Emission.png",
                "Tex_MetallicSmoothness.png"
            ]
        );
        for w in &report.written {
            let image = template.image(&w.suffix).unwrap();
            let (width, height, pixels) = read_png(&w.path);
            assert_eq!((width, height), (16, 12));
            assert_eq!(
                pixels,
                build(&doc, image, None, u64::MAX).unwrap(),
                "{}",
                w.file_name
            );
            assert!(!w.replaced);
            assert_eq!((w.width, w.height), (16, 12));
        }
        let albedo = &report.written[0];
        assert!(albedo.srgb && albedo.alpha_is_transparency && !albedo.normal_map);
        let packed = &report.written[1];
        assert!(!packed.srgb && !packed.alpha_is_transparency && !packed.normal_map);
        assert_eq!(packed.kind, ExportImageKind::Packed);
        let emission = &report.written[2];
        assert!(emission.srgb && !emission.alpha_is_transparency);
    }

    #[test]
    fn a_painted_normal_is_flagged_as_a_normal_map_and_ao_picks_more_images() {
        let s = Scratch::new();
        let mut doc = Document::with_tile_size(16, 12, 8).unwrap();
        let id = doc.add_layer("n").unwrap();
        paint(&mut doc, id, Channel::Normal, 9);
        doc.clear_history().unwrap();
        let occlusion: Vec<u8> = (0..16 * 12).map(|i| (i * 3) as u8).collect();
        let mut set = set_export(&doc);
        set.occlusion = Some(&occlusion);
        let hdrp = ExportTemplate::unity_hdrp();
        let report = write_template(&s.out(), &hdrp, &set, &WriteOptions::default()).unwrap();
        let names: Vec<&str> = report
            .written
            .iter()
            .map(|w| w.file_name.as_str())
            .collect();
        assert_eq!(
            names,
            ["Tex_BaseColor.png", "Tex_MaskMap.png", "Tex_Normal.png"]
        );
        let normal = report.written.last().unwrap();
        assert!(normal.normal_map && !normal.srgb);
        let (_, _, mask) = read_png(&report.written[1].path);
        assert_eq!(
            mask,
            build(
                &doc,
                hdrp.image("MaskMap").unwrap(),
                Some(&occlusion),
                u64::MAX
            )
            .unwrap()
        );
        assert_eq!(mask[1], occlusion[0], "G は焼いた AO");
    }

    #[test]
    fn padding_is_applied_between_build_and_write() {
        let s = Scratch::new();
        let doc = document();
        let mut coverage = vec![false; 16 * 12];
        for y in 3..7 {
            for x in 4..9 {
                coverage[y * 16 + x] = true;
            }
        }
        let template = standard();
        let mut set = set_export(&doc);
        set.padding = Some(PaddingSpec {
            coverage: &coverage,
            reach: Reach::Texels(2),
        });
        let report = write_template(&s.out(), &template, &set, &WriteOptions::default()).unwrap();
        assert_eq!(report.padding, PaddingOutcome::Applied);
        for w in &report.written {
            let image = template.image(&w.suffix).unwrap();
            let built = build(&doc, image, None, u64::MAX).unwrap();
            let want =
                padding::dilate(&built, 16, 12, &coverage, Reach::Texels(2), u64::MAX).unwrap();
            assert_ne!(want, built, "塗り広げで変わる画像のはず: {}", w.file_name);
            assert_eq!(read_png(&w.path).2, want, "{}", w.file_name);
        }
        // 覆いが空なら掛けない。大きさが違えば断る
        let empty = vec![false; 16 * 12];
        set.padding = Some(PaddingSpec {
            coverage: &empty,
            reach: Reach::Fill,
        });
        let s2 = Scratch::new();
        let report = write_template(&s2.out(), &template, &set, &WriteOptions::default()).unwrap();
        assert_eq!(report.padding, PaddingOutcome::NoCoverage);
        assert_eq!(
            read_png(&report.written[0].path).2,
            build(&doc, &template.images[0], None, u64::MAX).unwrap()
        );
        set.padding = Some(PaddingSpec {
            coverage: &empty[1..],
            reach: Reach::Fill,
        });
        let s3 = Scratch::new();
        assert!(matches!(
            write_template(&s3.out(), &template, &set, &WriteOptions::default()),
            Err(ExportError::Calc(CalcError::InvalidArgument(_)))
        ));
        assert!(s3.names().is_empty());
    }

    #[test]
    fn existing_files_are_refused_by_default_and_replaced_on_request() {
        let s = Scratch::new();
        let doc = document();
        let template = standard();
        fs::create_dir_all(s.out()).unwrap();
        let old = s.out().join("Tex_MetallicSmoothness.png");
        fs::write(&old, b"old").unwrap();
        let err = write_template(
            &s.out(),
            &template,
            &set_export(&doc),
            &WriteOptions::default(),
        )
        .unwrap_err();
        let ExportError::WouldReplace(paths) = err else {
            panic!("置き換えない設定のはず");
        };
        assert_eq!(paths, vec![old.clone()]);
        assert_eq!(fs::read(&old).unwrap(), b"old");
        assert_eq!(
            s.names(),
            ["Tex_MetallicSmoothness.png"],
            "断ったら何も作らない"
        );
        assert_eq!(
            existing_files(&s.out(), ["Tex_Albedo.png", "Tex_MetallicSmoothness.png"]),
            vec![old.clone()]
        );
        let options = WriteOptions {
            overwrite: Overwrite::Replace,
            cancel: None,
        };
        let report = write_template(&s.out(), &template, &set_export(&doc), &options).unwrap();
        let replaced: Vec<(&str, bool)> = report
            .written
            .iter()
            .map(|w| (w.suffix.as_str(), w.replaced))
            .collect();
        assert_eq!(
            replaced,
            [
                ("Albedo", false),
                ("MetallicSmoothness", true),
                ("Emission", false)
            ]
        );
        assert_eq!(read_png(&old).0, 16);
        assert_eq!(s.names().len(), 3, "一時ファイルは残らない");
    }

    #[test]
    fn nothing_to_write_creates_nothing() {
        let s = Scratch::new();
        let doc = Document::with_tile_size(16, 12, 8).unwrap();
        let report = write_template(
            &s.out(),
            &standard(),
            &set_export(&doc),
            &WriteOptions::default(),
        )
        .unwrap();
        assert!(report.written.is_empty());
        assert_eq!(report.skipped.len(), 6);
        assert!(!s.out().exists());
    }

    #[test]
    fn a_working_budget_refusal_writes_nothing() {
        let s = Scratch::new();
        let doc = document();
        let mut set = set_export(&doc);
        set.max_working_bytes = 100;
        let err =
            write_template(&s.out(), &standard(), &set, &WriteOptions::default()).unwrap_err();
        assert!(matches!(
            err,
            ExportError::Calc(CalcError::WorkingBudgetExceeded { .. })
        ));
        assert!(!s.out().exists(), "フォルダも作らない");
    }

    fn albedo() -> ExportImage {
        ExportImage::of("Albedo", ExportImageKind::BaseColor)
    }
    fn normal() -> ExportImage {
        ExportImage::of("Normal", ExportImageKind::Normal)
    }

    fn files<'a>(images: &'a [ExportImage], names: &[&str], w: u32, h: u32) -> Vec<ExportFile<'a>> {
        images
            .iter()
            .zip(names)
            .map(|(image, name)| ExportFile {
                name: (*name).to_string(),
                image,
                width: w,
                height: h,
            })
            .collect()
    }

    fn solid(w: u32, h: u32, v: u8) -> Vec<u8> {
        vec![v; w as usize * h as usize * 4]
    }

    #[test]
    fn name_and_size_problems_are_refused_before_anything_is_made() {
        let s = Scratch::new();
        let images = [albedo(), normal()];
        let mut made = 0;
        let mut produce = |_: usize| {
            made += 1;
            Ok(solid(2, 2, 1))
        };
        let run =
            |names: &[&str],
             w: u32,
             h: u32,
             produce: &mut dyn FnMut(usize) -> Result<Vec<u8>, ExportError>| {
                write_images_inner(
                    &s.out(),
                    &files(&images, names, w, h),
                    &WriteOptions::default(),
                    produce,
                    &mut |_, _| Ok(()),
                )
            };
        assert!(matches!(
            run(&["a.png", "A.PNG"], 2, 2, &mut produce),
            Err(ExportError::NameClash(_))
        ));
        assert!(matches!(
            run(&["a/b.png", "c.png"], 2, 2, &mut produce),
            Err(ExportError::InvalidName(_))
        ));
        assert!(matches!(
            run(&["", "c.png"], 2, 2, &mut produce),
            Err(ExportError::InvalidName(_))
        ));
        assert!(matches!(
            run(&["..", "c.png"], 2, 2, &mut produce),
            Err(ExportError::InvalidName(_))
        ));
        assert!(matches!(
            run(&["a.png", "b.png"], 0, 2, &mut produce),
            Err(ExportError::InvalidImage(_))
        ));
        assert!(matches!(
            run(&["a.png", "b.png"], MAX_SIDE + 1, 2, &mut produce),
            Err(ExportError::InvalidImage(_))
        ));
        assert_eq!(made, 0);
        assert!(!s.out().exists());
        // 中身の長さが違えば、そこで断り、置いた一時ファイルも消す
        let mut short = |i: usize| Ok(if i == 0 { solid(2, 2, 1) } else { vec![0u8; 3] });
        assert!(matches!(
            run(&["a.png", "b.png"], 2, 2, &mut short),
            Err(ExportError::InvalidImage(_))
        ));
        assert!(s.names().is_empty());
    }

    #[test]
    fn directories_and_links_are_not_replaced() {
        let s = Scratch::new();
        fs::create_dir_all(s.out().join("a.png")).unwrap();
        let images = [albedo()];
        let options = WriteOptions {
            overwrite: Overwrite::Replace,
            cancel: None,
        };
        let err = write_images(
            &s.out(),
            &files(&images, &["a.png"], 2, 2),
            &options,
            |_| Ok(solid(2, 2, 1)),
        )
        .unwrap_err();
        assert!(matches!(err, ExportError::NotAFile(_)));
        assert!(s.out().join("a.png").is_dir());
        #[cfg(unix)]
        {
            let target = s.0.join("elsewhere.png");
            fs::write(&target, b"x").unwrap();
            std::os::unix::fs::symlink(&target, s.out().join("b.png")).unwrap();
            let images = [albedo()];
            let err = write_images(
                &s.out(),
                &files(&images, &["b.png"], 2, 2),
                &options,
                |_| Ok(solid(2, 2, 1)),
            )
            .unwrap_err();
            assert!(matches!(err, ExportError::NotAFile(_)));
            assert_eq!(fs::read(&target).unwrap(), b"x", "リンクの先は書き換えない");
        }
    }

    /// 段ごとに失敗を差し込む。置き換えの前（flushed・verified・before-commit）なら、元のファイルは 1 つも変わらず新しいファイルも一時ファイルも残らない。
    #[test]
    fn a_failure_before_the_first_replacement_changes_nothing() {
        for (stage, at) in [
            ("flushed", 0),
            ("flushed", 1),
            ("verified", 0),
            ("verified", 2),
            ("before-commit", 0),
        ] {
            let s = Scratch::new();
            fs::create_dir_all(s.out()).unwrap();
            fs::write(s.out().join("a.png"), b"old a").unwrap();
            fs::write(s.out().join("c.png"), b"old c").unwrap();
            let images = [albedo(), normal(), albedo()];
            let list = files(&images, &["a.png", "b.png", "c.png"], 4, 3);
            let mut seen = 0;
            let result = write_images_inner(
                &s.out(),
                &list,
                &WriteOptions {
                    overwrite: Overwrite::Replace,
                    cancel: None,
                },
                &mut |i| Ok(solid(4, 3, i as u8 + 1)),
                &mut |phase, _| {
                    if phase == stage {
                        if seen == at {
                            return Err(ExportError::Io("注入した障害".into()));
                        }
                        seen += 1;
                    }
                    Ok(())
                },
            );
            assert!(matches!(result, Err(ExportError::Io(_))), "{stage}/{at}");
            assert_eq!(
                fs::read(s.out().join("a.png")).unwrap(),
                b"old a",
                "{stage}/{at}"
            );
            assert_eq!(
                fs::read(s.out().join("c.png")).unwrap(),
                b"old c",
                "{stage}/{at}"
            );
            assert_eq!(
                s.names(),
                ["a.png", "c.png"],
                "{stage}/{at}: 新しいファイルも一時ファイルも残さない"
            );
        }
    }

    #[test]
    fn a_corrupted_temporary_file_is_caught_by_the_read_back() {
        for damage in ["garbage", "truncate", "flip-a-pixel"] {
            let s = Scratch::new();
            fs::create_dir_all(s.out()).unwrap();
            fs::write(s.out().join("a.png"), b"old a").unwrap();
            let images = [albedo()];
            let result = write_images_inner(
                &s.out(),
                &files(&images, &["a.png"], 4, 3),
                &WriteOptions {
                    overwrite: Overwrite::Replace,
                    cancel: None,
                },
                &mut |_| Ok(solid(4, 3, 9)),
                &mut |phase, temp| {
                    if phase == "flushed" {
                        let bytes = fs::read(temp).unwrap();
                        match damage {
                            "garbage" => fs::write(temp, b"not a png").unwrap(),
                            "truncate" => fs::write(temp, &bytes[..bytes.len() / 2]).unwrap(),
                            _ => {
                                // 別の画像の PNG に差し替える（PNG としては正しいが画素が違う）
                                fs::write(
                                    temp,
                                    crate::composite_png::encode(&solid(4, 3, 10), 4, 3).unwrap(),
                                )
                                .unwrap();
                            }
                        }
                    }
                    Ok(())
                },
            );
            assert!(
                matches!(result, Err(ExportError::Verify(_))),
                "{damage}: {result:?}"
            );
            assert_eq!(
                fs::read(s.out().join("a.png")).unwrap(),
                b"old a",
                "{damage}"
            );
            assert_eq!(s.names(), ["a.png"], "{damage}");
        }
    }

    #[test]
    fn cancelling_before_or_during_the_run_replaces_nothing() {
        let s = Scratch::new();
        fs::create_dir_all(s.out()).unwrap();
        fs::write(s.out().join("a.png"), b"old a").unwrap();
        let images = [albedo(), normal()];
        let list = files(&images, &["a.png", "b.png"], 4, 3);
        let flag = AtomicBool::new(true);
        let options = WriteOptions {
            overwrite: Overwrite::Replace,
            cancel: Some(&flag),
        };
        let mut made = 0;
        let result = write_images(&s.out(), &list, &options, |_| {
            made += 1;
            Ok(solid(4, 3, 1))
        });
        assert!(matches!(result, Err(ExportError::Cancelled)));
        assert_eq!(made, 0, "始める前に立っていれば、何も作らない");
        // 1 枚目を作った後に立つ: 2 枚目の前で止まり、一時ファイルも消える
        let flag = AtomicBool::new(false);
        let options = WriteOptions {
            overwrite: Overwrite::Replace,
            cancel: Some(&flag),
        };
        let result = write_images(&s.out(), &list, &options, |i| {
            if i == 0 {
                flag.store(true, Ordering::Relaxed);
            }
            Ok(solid(4, 3, 1))
        });
        assert!(matches!(result, Err(ExportError::Cancelled)));
        assert_eq!(s.names(), ["a.png"]);
        assert_eq!(fs::read(s.out().join("a.png")).unwrap(), b"old a");
        // 最後の 1 枚を作った後に立っても、置き換えの前なら取り消せる
        let flag = AtomicBool::new(false);
        let options = WriteOptions {
            overwrite: Overwrite::Replace,
            cancel: Some(&flag),
        };
        let result = write_images(&s.out(), &list, &options, |i| {
            if i == 1 {
                flag.store(true, Ordering::Relaxed);
            }
            Ok(solid(4, 3, 1))
        });
        assert!(matches!(result, Err(ExportError::Cancelled)));
        assert_eq!(s.names(), ["a.png"]);
        assert_eq!(fs::read(s.out().join("a.png")).unwrap(), b"old a");
    }

    #[test]
    fn a_failure_while_replacing_reports_what_was_replaced() {
        let s = Scratch::new();
        fs::create_dir_all(s.out()).unwrap();
        for n in ["a.png", "b.png", "c.png"] {
            fs::write(s.out().join(n), format!("old {n}")).unwrap();
        }
        let images = [albedo(), normal(), albedo()];
        let list = files(&images, &["a.png", "b.png", "c.png"], 4, 3);
        let mut index = 0;
        let result = write_images_inner(
            &s.out(),
            &list,
            &WriteOptions {
                overwrite: Overwrite::Replace,
                cancel: None,
            },
            &mut |i| Ok(solid(4, 3, i as u8 + 1)),
            &mut |phase, _| {
                if phase == "before-rename" {
                    index += 1;
                    if index == 2 {
                        return Err(ExportError::Io("注入した障害".into()));
                    }
                }
                Ok(())
            },
        );
        let Err(ExportError::Partial { written, cause }) = result else {
            panic!("途中で失敗したことが分かるはず: {result:?}");
        };
        assert!(cause.contains("注入した障害"));
        assert_eq!(written.len(), 1);
        assert_eq!(written[0].file_name, "a.png");
        assert!(written[0].replaced);
        assert_eq!(read_png(&s.out().join("a.png")).2, solid(4, 3, 1));
        assert_eq!(fs::read(s.out().join("b.png")).unwrap(), b"old b.png");
        assert_eq!(fs::read(s.out().join("c.png")).unwrap(), b"old c.png");
        assert_eq!(
            s.names(),
            ["a.png", "b.png", "c.png"],
            "一時ファイルは残らない"
        );
    }

    #[test]
    fn a_failure_while_creating_new_files_removes_the_ones_already_created() {
        let s = Scratch::new();
        let images = [albedo(), normal(), albedo()];
        let list = files(&images, &["a.png", "b.png", "c.png"], 4, 3);
        let mut index = 0;
        let result = write_images_inner(
            &s.out(),
            &list,
            &WriteOptions::default(),
            &mut |_| Ok(solid(4, 3, 1)),
            &mut |phase, _| {
                if phase == "before-rename" {
                    index += 1;
                    if index == 3 {
                        return Err(ExportError::Io("注入した障害".into()));
                    }
                }
                Ok(())
            },
        );
        assert!(matches!(result, Err(ExportError::Io(_))), "{result:?}");
        assert!(
            s.names().is_empty(),
            "置き換えたものが無いので、全部が元通り: {:?}",
            s.names()
        );
    }

    #[test]
    fn a_file_created_before_the_last_check_is_refused() {
        let s = Scratch::new();
        let images = [albedo(), normal()];
        let list = files(&images, &["a.png", "b.png"], 4, 3);
        let theirs = s.out().join("b.png");
        let result = write_images_inner(
            &s.out(),
            &list,
            &WriteOptions::default(),
            &mut |_| Ok(solid(4, 3, 1)),
            &mut |phase, _| {
                if phase == "before-commit" {
                    fs::write(&theirs, b"theirs").unwrap();
                }
                Ok(())
            },
        );
        let Err(ExportError::WouldReplace(paths)) = result else {
            panic!("先にできたファイルは置き換えないはず: {result:?}");
        };
        assert_eq!(paths, vec![theirs.clone()]);
        assert_eq!(fs::read(&theirs).unwrap(), b"theirs");
        assert_eq!(
            s.names(),
            ["b.png"],
            "自分で作った a.png は消し、一時ファイルも残らない"
        );
    }

    #[test]
    fn a_file_created_between_the_check_and_the_link_is_not_overwritten() {
        // 調べた後（commit の確かめの後）、hard_link で置く前に他のプロセスが作ったファイル。素の rename なら上書きしてしまう
        let s = Scratch::new();
        let images = [albedo(), normal()];
        let list = files(&images, &["a.png", "b.png"], 4, 3);
        let theirs = s.out().join("b.png");
        let mut linked = 0;
        let result = write_images_inner(
            &s.out(),
            &list,
            &WriteOptions::default(),
            &mut |_| Ok(solid(4, 3, 1)),
            &mut |phase, target| {
                if phase == "before-link" {
                    linked += 1;
                    if target == theirs {
                        fs::write(&theirs, b"theirs").unwrap();
                    }
                }
                Ok(())
            },
        );
        let Err(ExportError::WouldReplace(paths)) = result else {
            panic!("先にできたファイルは置き換えないはず: {result:?}");
        };
        assert_eq!(paths, vec![theirs.clone()]);
        assert_eq!(
            linked, 2,
            "置かない設定では、置くたびに hard_link の前を通る"
        );
        assert_eq!(fs::read(&theirs).unwrap(), b"theirs");
        assert_eq!(
            s.names(),
            ["b.png"],
            "自分で作った a.png は消し、一時ファイルも残らない"
        );
    }

    #[test]
    fn replacing_on_request_does_not_go_through_the_link() {
        let s = Scratch::new();
        fs::create_dir_all(s.out()).unwrap();
        fs::write(s.out().join("a.png"), b"old a").unwrap();
        let images = [albedo()];
        let list = files(&images, &["a.png"], 4, 3);
        let mut seen = Vec::new();
        write_images_inner(
            &s.out(),
            &list,
            &WriteOptions {
                overwrite: Overwrite::Replace,
                cancel: None,
            },
            &mut |_| Ok(solid(4, 3, 1)),
            &mut |phase, _| {
                seen.push(phase.to_string());
                Ok(())
            },
        )
        .unwrap();
        assert!(!seen.iter().any(|p| p == "before-link"), "{seen:?}");
        assert_eq!(read_png(&s.out().join("a.png")).2, solid(4, 3, 1));
    }

    #[test]
    fn cancelling_during_the_padding_returns_cancelled_and_leaves_nothing() {
        let doc = document();
        let mut coverage = vec![false; 16 * 12];
        for y in 3..7 {
            for x in 4..9 {
                coverage[y * 16 + x] = true;
            }
        }
        let template = standard();
        let mut set = set_export(&doc);
        set.padding = Some(PaddingSpec {
            coverage: &coverage,
            reach: Reach::Fill,
        });
        // 何枚目の画像の塗り広げの前に旗が立っても、塗り広げの中で止まり、Calc に包まれず Cancelled で返る
        for raised_at in 0..3 {
            let s = Scratch::new();
            fs::create_dir_all(s.out()).unwrap();
            fs::write(s.out().join("Tex_Albedo.png"), b"old albedo").unwrap();
            let flag = AtomicBool::new(false);
            let options = WriteOptions {
                overwrite: Overwrite::Replace,
                cancel: Some(&flag),
            };
            let mut padded = 0;
            let result = write_template_inner(&s.out(), &template, &set, &options, &mut |i| {
                if i == raised_at {
                    flag.store(true, Ordering::Relaxed);
                }
                // 旗が立った後の塗り広げは、始まる前に止まる
                if !flag.load(Ordering::Relaxed) {
                    padded += 1;
                }
            });
            assert!(
                matches!(result, Err(ExportError::Cancelled)),
                "{raised_at} 枚目: {result:?}"
            );
            assert_eq!(padded, raised_at, "旗の前の画像は塗り広げ終えている");
            assert_eq!(
                s.names(),
                ["Tex_Albedo.png"],
                "{raised_at} 枚目: 一時ファイルも新しいファイルも残らない"
            );
            assert_eq!(
                fs::read(s.out().join("Tex_Albedo.png")).unwrap(),
                b"old albedo",
                "{raised_at} 枚目: 元のファイルは変わらない"
            );
        }
        // 旗が立たなければ最後まで書ける（上の試験の止まり方が旗のせいだと分かる）
        let s = Scratch::new();
        let flag = AtomicBool::new(false);
        let options = WriteOptions {
            overwrite: Overwrite::Refuse,
            cancel: Some(&flag),
        };
        let report =
            write_template_inner(&s.out(), &template, &set, &options, &mut |_| {}).unwrap();
        assert_eq!(report.written.len(), 3);
    }

    #[test]
    fn a_calculation_cancel_maps_to_the_export_cancel() {
        assert!(matches!(
            ExportError::from(CalcError::Cancelled),
            ExportError::Cancelled
        ));
        assert!(matches!(
            ExportError::from(CalcError::InvalidArgument("x")),
            ExportError::Calc(CalcError::InvalidArgument("x"))
        ));
    }

    #[test]
    fn a_name_at_the_length_limit_is_written_and_one_byte_more_is_refused_up_front() {
        // 一時ファイルの名前は行き先の名前に依らないので、上限いっぱいの名前でも重い仕事の後に落ちない
        let doc = document();
        let template = standard();
        let long_stem = format!("{}a", "あ".repeat(77)); // 232 バイト + "_MetallicSmoothness.png"（23 バイト）= 255
        let s = Scratch::new();
        let mut set = set_export(&doc);
        set.stem = &long_stem;
        let report = write_template(&s.out(), &template, &set, &WriteOptions::default()).unwrap();
        let longest = format!("{long_stem}_MetallicSmoothness.png");
        assert_eq!(longest.len(), 255);
        assert_eq!(report.written.len(), 3);
        assert!(report.written.iter().any(|w| w.file_name == longest));
        assert!(s.names().contains(&longest));
        assert!(s.names().iter().all(|n| !n.ends_with("pending~")));
        // 1 バイト多い名前は、何も作る前に断る
        let too_long = format!("{long_stem}a");
        let s2 = Scratch::new();
        set.stem = &too_long;
        assert!(matches!(
            write_template(&s2.out(), &template, &set, &WriteOptions::default()),
            Err(ExportError::InvalidName(_))
        ));
        assert!(!s2.out().exists());
    }

    #[test]
    fn the_temp_file_name_does_not_depend_on_the_target_name() {
        let s = Scratch::new();
        let images = [albedo()];
        let name = format!("{}.png", "n".repeat(251)); // 255 バイト
        let list = files(&images, &[name.as_str()], 4, 3);
        let mut temps = Vec::new();
        write_images_inner(
            &s.out(),
            &list,
            &WriteOptions::default(),
            &mut |_| Ok(solid(4, 3, 1)),
            &mut |phase, path| {
                if phase == "flushed" {
                    temps.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(temps.len(), 1);
        assert!(
            temps[0].len() < 64 && temps[0].ends_with(".pending~"),
            "{temps:?}"
        );
        assert_eq!(s.names(), [name]);
    }

    #[test]
    fn plan_lists_the_images_each_set_reads() {
        let doc = document();
        let empty = Document::with_tile_size(16, 12, 8).unwrap();
        let sets = [
            PlanSet {
                name: Some("Body"),
                document: &doc,
                has_occlusion: false,
            },
            PlanSet {
                name: Some("Hair"),
                document: &empty,
                has_occlusion: true,
            },
        ];
        let plan = plan_template("Tex", &sets, &standard()).unwrap();
        let names: Vec<(usize, &str)> =
            plan.iter().map(|p| (p.set, p.file_name.as_str())).collect();
        assert_eq!(
            names,
            [
                (0, "Tex_Body_Albedo.png"),
                (0, "Tex_Body_MetallicSmoothness.png"),
                (0, "Tex_Body_Emission.png"),
                (1, "Tex_Hair_Occlusion.png"),
            ]
        );
        let same = [
            sets[0],
            PlanSet {
                name: Some("body"),
                ..sets[0]
            },
        ];
        let plan = plan_template("Tex", &same, &standard()).unwrap();
        assert_eq!(clashes(plan.iter().map(|p| p.file_name.as_str())).len(), 3);
    }
}
