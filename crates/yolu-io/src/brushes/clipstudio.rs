//! CLIP STUDIO PAINT のサブツールのフォルダを探して、見つかった `.sut` を名前と筆先の見本で一覧にする。**読むだけ**: 探す所は
//! ディレクトリの一覧とファイルの大きさだけを読み、`.sut` の中身は `peek`・取り込みが「読む」。どちらも CLIP STUDIO のフォルダへ
//! 書かず・ファイルを動かさず・消さず・そばに作業用のファイルを作らない（読むのは共有の読み取りでメモリへ取る。下の `read_stable`）。
//!
//! # フォルダの見つけ方
//! CELSYS の設定のフォルダの場所は、公式の案内（CLIP STUDIO ASK・サポートの回答）では、Windows で
//! `C:\Users\<名前>\AppData\Roaming\CELSYSUserData\CELSYS`（V1.10.13 以降）か、`C:\Users\<名前>\Documents\CELSYS`（それ以前。
//! OneDrive が書類を同期していれば `OneDrive\Documents\CELSYS`）。ここまでは公開の情報で確かめた。
//! その下で `.sut` がどのフォルダに置かれるか（`CLIPStudioModule\SubTool\…`・版ごとのフォルダ `CLIPStudioPaintVer…`・
//! ダウンロードした素材の `CLIPStudioCommon\Material\…`）は、公開の情報で確かめられなかった。そこで下のフォルダの名前を決め打ちにせず、
//! 設定のフォルダの下を幅優先で（浅い所から）数えて、拡張子 `.sut` のファイルを集める（深さ・見るものの数・集める数に上限）。
//! 見つからない・読めないときは理由（`Missing`）を返し、フォルダを手で選んでも同じ探し方で一覧にする。
//! 実機の Windows では確かめていない（docs/BRUSH_IMPORT.md の「確かめていないこと」）。

use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use yolu_core::{Brush, BrushTip};

use super::{import_bytes, pretty_name, BrushImportError, FileKind, MAX_SUT_BYTES};

/// 環境から取ったフォルダの手がかり。試験は環境変数を使わず、これを直接作る。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    /// Windows の `APPDATA`（`…\AppData\Roaming`）。
    pub appdata: Option<PathBuf>,
    /// 利用者のフォルダ（Windows の `USERPROFILE`。無ければ `HOME`）。
    pub profile: Option<PathBuf>,
    /// OneDrive のフォルダ（Windows の `OneDrive`）。
    pub onedrive: Option<PathBuf>,
}

impl Places {
    /// 今の環境変数から。
    pub fn from_env() -> Places {
        let var = |name: &str| {
            std::env::var_os(name)
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        Places {
            appdata: var("APPDATA"),
            profile: var("USERPROFILE").or_else(|| var("HOME")),
            onedrive: var("OneDrive"),
        }
    }

    /// CELSYS の設定のフォルダの候補（新しい版の場所から。存在は確かめない）。
    pub fn celsys_folders(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let mut push = |p: PathBuf| {
            if !out.contains(&p) {
                out.push(p);
            }
        };
        if let Some(appdata) = &self.appdata {
            push(appdata.join("CELSYSUserData").join("CELSYS"));
        }
        if let Some(profile) = &self.profile {
            push(profile.join("Documents").join("CELSYS"));
        }
        if let Some(onedrive) = &self.onedrive {
            push(onedrive.join("Documents").join("CELSYS"));
        }
        out
    }
}

/// 探す範囲の上限（設定のフォルダの下の素材は数万ファイルになりうる）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// フォルダの深さ（探し始めのフォルダが 0）。
    pub depth: usize,
    /// 見るディレクトリの項目の数の合計。
    pub entries: usize,
    /// 集める `.sut` の数。
    pub files: usize,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            depth: 8,
            entries: 60_000,
            files: 1_000,
        }
    }
}

/// 見つけた `.sut` 1 つ。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    /// ファイルの大きさ（バイト）。
    pub size: u64,
}

/// `.sut` が見つからなかった理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// 探すフォルダが 1 つも無い。
    NoFolder,
    /// フォルダはあるが、開けない（権限など）。
    Unreadable,
    /// フォルダは読めたが、`.sut` が無い。
    NoFiles,
}

/// 探した結果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Scan {
    /// 見つけた `.sut`（パスの小文字の順）。
    pub files: Vec<Found>,
    /// 実際に開けて探したフォルダ。
    pub searched: Vec<PathBuf>,
    /// 上限に達して、まだ見ていない所が残った。
    pub truncated: bool,
    /// 1 つも見つからなかったときの理由。
    pub missing: Option<Missing>,
}

fn is_sut(name: &std::ffi::OsStr) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("sut"))
}

/// フォルダの下から `.sut` を幅優先で集める。ディレクトリの一覧と各ファイルの大きさだけを読む（シンボリックリンクはたどらない）。
/// 上限（見る項目の数・集める数）に達したらそこで止め、`truncated` を立てる（深さの上限で入らなかったフォルダも）。
pub fn scan(folders: &[PathBuf], limits: Limits) -> Scan {
    let mut out = Scan::default();
    let mut denied = false;
    let mut budget = limits.entries;
    'roots: for root in folders {
        match std::fs::read_dir(root) {
            Ok(_) => out.searched.push(root.clone()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
            Err(_) => {
                denied = true;
                continue;
            }
        }
        let mut queue: VecDeque<(PathBuf, usize)> = VecDeque::from([(root.clone(), 0)]);
        while let Some((dir, depth)) = queue.pop_front() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if budget == 0 || out.files.len() >= limits.files {
                    out.truncated = true;
                    break 'roots;
                }
                budget -= 1;
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                if kind.is_dir() {
                    if depth < limits.depth {
                        queue.push_back((entry.path(), depth + 1));
                    } else {
                        out.truncated = true;
                    }
                } else if kind.is_file() && is_sut(&entry.file_name()) {
                    if let Ok(meta) = entry.metadata() {
                        out.files.push(Found {
                            path: entry.path(),
                            size: meta.len(),
                        });
                    }
                }
            }
        }
    }
    out.files
        .sort_by_key(|f| f.path.to_string_lossy().to_lowercase());
    out.files.dedup_by(|a, b| a.path == b.path);
    if out.files.is_empty() {
        out.missing = Some(if !out.searched.is_empty() {
            Missing::NoFiles
        } else if denied {
            Missing::Unreadable
        } else {
            Missing::NoFolder
        });
    }
    out
}

/// CELSYS の設定のフォルダの既定の場所を探して、その下の `.sut` を集める。
pub fn find(places: &Places) -> Scan {
    scan(&places.celsys_folders(), Limits::default())
}

/// ファイルを共有の読み取りでメモリへ取る。**書き込みも作業用のファイルも作らない**。CLIP STUDIO が開いている間でも、書き込みの
/// 最中でなければ読める（Windows の Rust の標準のファイルの開き方は、ほかの読み書きと共有する。SQLite の鍵は範囲の鍵で、先頭の
/// 読みを妨げない）。書き込みの途中の本体は確定前の中身を含みうるので、次の 2 つを両方満たした中身だけを返す。
/// - 読む前後で、確定していないジャーナルが残っていない（`journal_pending`）。書き手はキャッシュが溢れたりコミットの最中だったり
///   すると、まだ確定していないページを先に本体へ書く（元のページは `<path>-journal` へ退避する）。その書き込みが読み始める前に
///   済んで止まっていると、本体の大きさも更新時刻も読む間は変わらず、下の確かめでは見分けられない（コミットの最中に落ちて、
///   未処理のジャーナルが残った場合も同じ）。SQLite が自分で読むときは、この残りを元に戻してから読むが、こちらは書かないので、
///   戻さず断る。
/// - 読む前後でファイルの大きさと更新時刻が同じ。
///
/// どちらかが外れたら、少し待って取り直す（3 回まで。それでも外れるなら `ResourceBusy`）。書き込みの記録（`-wal`）のファイルは
/// 読まないので、まだ書き戻されていない最新の変更は見えない（本体は書き戻し済みの確定した状態）。`limit` を超えるファイルは
/// 読まずに `FileTooLarge`。
pub fn read_stable(path: &Path, limit: u64) -> Result<Vec<u8>, BrushImportError> {
    read_stable_with(path, limit, &mut |_| {})
}

/// 取り直す回数。
const READ_ATTEMPTS: usize = 3;
/// 取り直すまでの待ち（回数に比例して延ばす）。書き手のコミットは、ふつうこの間に終わる。
const READ_RETRY_WAIT: std::time::Duration = std::time::Duration::from_millis(40);

/// ジャーナルのファイルの場所（本体のファイル名の末尾に `-journal`）。
fn journal_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push("-journal");
    PathBuf::from(name)
}

/// 本体に確定前のページがありうるジャーナルが残っているか（開いて先頭の 1 バイトを読むだけ）。SQLite 自身の判定（`hasHotJournal`）と
/// 同じく、無い・空・先頭の 1 バイトが 0 なら「ない」。先頭の印は、書き手がジャーナルの中身を同期して初めて書かれ（それまでは 0。
/// 本体のページを上書きするのは同期の後なので、0 の間は本体は確定済みのまま）、TRUNCATE・PERSIST のモードでは終えたときに 0 へ
/// 戻される。したがって、小さな書き込みの最中（まだ同期していない）でも読め、確定前のページを書き出した後だけを断れる。
/// 開けない・読めないときは（書き手が掴んでいるなど）見分けられないので、残っているものとして扱う。
fn journal_pending(path: &Path) -> bool {
    let mut file = match File::open(journal_path(path)) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return false,
        Err(_) => return true,
    };
    let mut first = [0u8; 1];
    match file.read(&mut first) {
        Ok(0) => false,
        Ok(_) => first[0] != 0,
        Err(_) => true,
    }
}

/// `read_stable`。`after_read` は、中身を読み終えて、最後の確かめをする前に呼ぶ（試験が、読んでいる間の書き込みを再現する）。
pub(crate) fn read_stable_with(
    path: &Path,
    limit: u64,
    after_read: &mut dyn FnMut(usize),
) -> Result<Vec<u8>, BrushImportError> {
    for attempt in 0..READ_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(READ_RETRY_WAIT * attempt as u32);
        }
        let mut file = File::open(path)?;
        let journal_before = journal_pending(path);
        let before = file.metadata()?;
        if before.len() > limit {
            return Err(BrushImportError::FileTooLarge { limit });
        }
        let mut bytes = Vec::with_capacity(before.len() as usize);
        // 読んでいる間に伸びたファイルでも上限を超えて読まない
        (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(BrushImportError::FileTooLarge { limit });
        }
        after_read(attempt);
        let after = std::fs::metadata(path)?;
        if !journal_before
            && after.len() == before.len()
            && after.modified().ok() == before.modified().ok()
            && bytes.len() as u64 == before.len()
            && !journal_pending(path)
        {
            return Ok(bytes);
        }
    }
    Err(io::Error::from(io::ErrorKind::ResourceBusy).into())
}

/// 一覧に出す筆先の見本（正方形。行は上から。255 が塗る）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TipPreview {
    pub side: u32,
    pub alpha: Vec<u8>,
}

/// 見本の 1 辺。
pub const PREVIEW_SIDE: u32 = 64;

/// `.sut` の中身（名前と筆先の見本）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Peek {
    /// ブラシの名前（最初の 8 つまで）。
    pub names: Vec<String>,
    /// ファイルの中のブラシの数。
    pub brushes: usize,
    /// 最初のブラシの筆先の見本。
    pub preview: TipPreview,
}

/// `.sut` を読んで、名前と筆先の見本を取る（取り込みと同じ読み方。何も書かない）。
pub fn peek(path: &Path) -> Result<Peek, BrushImportError> {
    let bytes = read_stable(path, MAX_SUT_BYTES)?;
    let name = path.file_name().map(|n| pretty_name(&n.to_string_lossy()));
    let set = import_bytes(FileKind::Sut, &bytes, name.as_deref())?;
    let preview = set
        .brushes
        .first()
        .map(|b| preview_of(&b.brush))
        .unwrap_or_else(|| preview_of(&Brush::default()));
    Ok(Peek {
        names: set.brushes.iter().take(8).map(|b| b.name.clone()).collect(),
        brushes: set.brushes.len(),
        preview,
    })
}

/// ブラシの筆先の見本。画像の筆先は縦横比を保って枠に収め（拡大も縮小も面積の平均）、丸い筆先は硬さで縁をぼかした円。
pub fn preview_of(brush: &Brush) -> TipPreview {
    let side = PREVIEW_SIDE;
    let tip = brush
        .tip
        .image
        .as_ref()
        .or_else(|| brush.tip.images.first());
    let alpha = match tip {
        Some(tip) => image_preview(tip, side),
        None => disc_preview(brush.base.hardness, brush.tip.roundness, side),
    };
    TipPreview { side, alpha }
}

fn image_preview(tip: &BrushTip, side: u32) -> Vec<u8> {
    let (w, h) = (tip.width() as f64, tip.height() as f64);
    let mut out = vec![0u8; (side * side) as usize];
    if w < 1.0 || h < 1.0 {
        return out;
    }
    let scale = side as f64 / w.max(h);
    let (fit_w, fit_h) = (w * scale, h * scale);
    let (off_x, off_y) = ((side as f64 - fit_w) / 2.0, (side as f64 - fit_h) / 2.0);
    let inv = 1.0 / scale;
    for oy in 0..side {
        for ox in 0..side {
            let (px, py) = (ox as f64 - off_x, oy as f64 - off_y);
            if px + 1.0 <= 0.0 || py + 1.0 <= 0.0 || px >= fit_w || py >= fit_h {
                continue;
            }
            let (x0, x1) = ((px * inv).max(0.0), ((px + 1.0) * inv).min(w));
            // 画像の行は下から。見本の行は上から
            let (y0, y1) = ((py * inv).max(0.0), ((py + 1.0) * inv).min(h));
            let (mut sum, mut weight) = (0.0, 0.0);
            let mut y = y0.floor();
            while y < y1 {
                let wy = (y + 1.0).min(y1) - y.max(y0);
                let mut x = x0.floor();
                while x < x1 {
                    let wx = (x + 1.0).min(x1) - x.max(x0);
                    let row = tip.height() - 1 - (y as u32).min(tip.height() - 1);
                    let a = tip.at((x as u32).min(tip.width() - 1), row) as f64;
                    sum += a * wx * wy;
                    weight += wx * wy;
                    x += 1.0;
                }
                y += 1.0;
            }
            if weight > 0.0 {
                out[(oy * side + ox) as usize] = (sum / weight).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn disc_preview(hardness: f64, roundness: f64, side: u32) -> Vec<u8> {
    let half = side as f64 / 2.0;
    let radius = half - 1.0;
    let squash = roundness.clamp(0.01, 1.0);
    let hard = hardness.clamp(0.0, 1.0);
    let mut out = vec![0u8; (side * side) as usize];
    for oy in 0..side {
        for ox in 0..side {
            let dx = (ox as f64 + 0.5 - half) / radius;
            let dy = (oy as f64 + 0.5 - half) / (radius * squash);
            let r = (dx * dx + dy * dy).sqrt();
            let a = if r <= hard {
                1.0
            } else if r >= 1.0 {
                0.0
            } else {
                (1.0 - r) / (1.0 - hard)
            };
            out[(oy * side + ox) as usize] = (a * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/clipstudio-unit-tests")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("x.sut");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn touch(path: &Path, offset_secs: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(offset_secs))
            .unwrap();
    }

    #[test]
    fn a_file_that_changes_while_it_is_read_is_read_again() {
        let path = temp_file("changes", b"0123456789");
        let mut calls = 0;
        // 1 回目と 2 回目の読みの最中に、更新時刻が変わる（書き込みの途中を表す）。3 回目は静か
        let bytes = read_stable_with(&path, 1 << 20, &mut |attempt| {
            calls += 1;
            if attempt < 2 {
                touch(&path, 10 + attempt as u64);
            }
        })
        .unwrap();
        assert_eq!(bytes, b"0123456789");
        assert_eq!(calls, 3);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_file_that_never_settles_is_refused_instead_of_returning_a_torn_read() {
        let path = temp_file("never", b"abc");
        let result = read_stable_with(&path, 1 << 20, &mut |attempt| {
            std::fs::write(&path, vec![b'x'; 10 + attempt]).unwrap();
        });
        match result {
            Err(BrushImportError::Io(e)) => assert_eq!(e.kind(), io::ErrorKind::ResourceBusy),
            other => panic!("ResourceBusy のはず: {other:?}"),
        }
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_journal_that_appears_during_the_read_is_waited_out_and_the_next_try_is_returned() {
        let path = temp_file("journal-appears", b"0123456789");
        let journal = journal_path(&path);
        let mut calls = 0;
        // 1 回目の読みの最中に書き手が書き始めてジャーナルができる（本体の大きさ・更新時刻は変わらない）。2 回目の読みの最中に終えて消える
        let bytes = read_stable_with(&path, 1 << 20, &mut |attempt| {
            calls += 1;
            match attempt {
                0 => std::fs::write(&journal, [0xd9, 1, 2, 3]).unwrap(),
                1 => std::fs::remove_file(&journal).unwrap(),
                _ => {}
            }
        })
        .unwrap();
        assert_eq!(bytes, b"0123456789");
        assert_eq!(
            calls, 3,
            "ジャーナルが残っていた 2 回は捨てて、3 回目を返す"
        );
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn only_a_journal_that_is_not_finished_counts() {
        let path = temp_file("journal-kinds", b"abc");
        let journal = journal_path(&path);
        assert!(journal.ends_with("x.sut-journal"));
        assert!(!journal_pending(&path), "無い");
        std::fs::write(&journal, b"").unwrap();
        assert!(!journal_pending(&path), "空（DELETE・TRUNCATE で終えた後）");
        std::fs::write(&journal, [0u8; 512]).unwrap();
        assert!(
            !journal_pending(&path),
            "先頭が 0（PERSIST で無効にした後）"
        );
        std::fs::write(&journal, [0xd9, 0xd5, 0x05, 0xf9]).unwrap();
        assert!(journal_pending(&path), "先頭が 0 でない");
        // ファイルでなく、開けないもの（書き手が掴んでいる場合の代わり）は、確定済みとは言えない
        std::fs::remove_file(&journal).unwrap();
        std::fs::create_dir(&journal).unwrap();
        assert!(journal_pending(&path));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_file_over_the_limit_is_refused_and_one_at_the_limit_is_read() {
        let path = temp_file("grows", b"0123");
        assert!(matches!(
            read_stable(&path, 3),
            Err(BrushImportError::FileTooLarge { limit: 3 })
        ));
        assert_eq!(read_stable(&path, 4).unwrap(), b"0123");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
