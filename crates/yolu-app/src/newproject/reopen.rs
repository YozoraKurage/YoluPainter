//! プロジェクトのモデルのファイル（FBX）を .ylp の view.json に残し、開くと読み直す。
//!
//! - 残す形は .ylp からの相対のパス（'/' 区切り。同じ場所から辿れなければ絶対のパス）。プロジェクトとモデルを同じフォルダの
//!   組で動かしても、参照は切れない。形式も正本の版も変えない（`yolu_io::Project::with_view_model`。view.json の任意の項目）。
//! - 退避のフォルダー（`<名前>.ylp-backups~`）の中の退避は、退避した元の .ylp の前の版なので、パスは元の .ylp のあるフォルダーから解く
//!   （`base_dir`。保存で書き直す相対のパスも同じ基準）。探すのはこの 1 か所だけで、見つからなければ「モデルが見つかりません」にする
//!   （退避の場所から解いた道を 2 番目に探すと、同じ名前の別のモデルを黙って拾いうる）。
//! - 開くとき: ファイルがあるかの確かめも、読むのも別のスレッドで行い（画面のスレッドはファイルに触らない。終わるまで描ける。
//!   取消は仕事の札）、読み終えたら 3D ビューに入れ、セットを鍵で結び付ける（モデルのマテリアルにセットを増やさない）。
//!   ファイルが無くても参照は残す（保存で失わず、構成の「読み直す」で見つかったときに読める）。
//! - ネットワークのパス（UNC・`\\?\UNC\`・デバイスのパス）で書かれた絶対のパスは、開くときに自動では触らない（参照だけ残す）。
//!   細工した .ylp が、開くだけで外のホストへ Windows の認証を送らせたり、応答しない共有で固まらせたりできないようにする。
//!   .ylp からの相対のパスと、ドライブ文字つきの絶対のパスは読む（マップしたネットワークドライブは見分けない: 細工した .ylp は
//!   利用者のドライブの割り当てを作れない）。構成の「読み直す」は利用者の操作なので、ネットワークのパスでも読める。新規プロジェクトの
//!   ウィンドウの初めのモデルには使わない（利用者が選んだモデルではない）。保存し直しても別の場所の参照には変わらない（'/' 区切りにそろえるだけ。.ylp と同じ共有なら Windows は相対にする）。
//! - 読み終えたら、ファイルのポーズ（根の `pose.json`）を戻す（`view3d::pose::stored`）。
//! - Live Link のモデル（Unity のシーンのもの）が付いているときは、モデルのファイルは読まない（参照だけ残す。ポーズも戻さず、ファイルのポーズは保存でも
//!   そのまま残る。Unity から受けるポーズは頂点の位置で、保存しない）。

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, TryRecvError};

use crate::model::SceneModel;
use crate::state::AppState;
use crate::view3d::pose::{self, PrepareJob};

/// .ylp を開いて、モデルを読んでいる最中。
pub struct Reopen {
    pub path: PathBuf,
    stage: Stage,
    /// 始めたときのプロジェクトの世代（変わったら結果を捨てる）。
    generation: u64,
    /// 開いたときの知らせ（読み終えたら、その後ろにモデルの知らせを足す）とその種類。
    base: String,
    base_kind: crate::notice::Kind,
}

/// 読み直しの段階。
enum Stage {
    /// ファイルがあるかを別のスレッドで確かめている（遅い共有でも画面を止めない）。
    Checking(Receiver<bool>),
    /// 別のスレッドで読んでいる。
    Loading(PrepareJob),
}

impl std::fmt::Debug for Reopen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reopen").field("path", &self.path).finish()
    }
}

impl Reopen {
    /// 取り消す（読み込みは止まり、結果は捨てる。確かめている段階は、結果を受けずに捨てる）。
    pub fn cancel(&self) {
        if let Stage::Loading(job) = &self.stage {
            job.cancel();
        }
    }
    /// 読んでいるファイルの名前。
    pub fn file_name(&self) -> String {
        file_name(&self.path)
    }
    /// 読み込みの進み具合（ファイルを確かめている間・まだ知らせが無い間は None）。
    pub fn fraction(&self) -> Option<f32> {
        match &self.stage {
            Stage::Loading(job) => job.fraction(),
            Stage::Checking(_) => None,
        }
    }
}

impl Drop for Reopen {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// 絶対にして、`.` と `..` をたたむ（ファイルが無くても使える。シンボリックリンクは辿らない）。
fn absolute(path: &Path) -> PathBuf {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// モデルのファイルを、.ylp のあるフォルダからの相対のパス（'/' 区切り）にする。同じ場所から辿れなければ（別のドライブなど）絶対のパス。
pub fn relative_model_path(model: &Path, ylp: &Path) -> String {
    let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
    // Windows 以外では、先頭の `//` が根にたたまれ、`\\host\share` は相対のファイル名になるので、成分をたどると別の参照に変わる。
    // ネットワークのパスは、書かれたまま（区切りを '/' にそろえるだけ）残す
    if !cfg!(windows) && is_network_path(&model.to_string_lossy()) {
        return slash(model);
    }
    let model = absolute(model);
    let base = absolute(&base_dir(ylp));
    let m: Vec<Component> = model.components().collect();
    let b: Vec<Component> = base.components().collect();
    let common = m.iter().zip(&b).take_while(|(x, y)| x == y).count();
    // ルート（ドライブ）が違えば辿れない
    if common == 0 || (common == 1 && matches!(m.first(), Some(Component::Prefix(_)))) {
        return slash(&model);
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..b.len() {
        parts.push("..".into());
    }
    for c in &m[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    parts.join("/")
}

/// view.json に残っているパスが、ネットワークのパスか（`\\host\share\…`・`//host/share/…`・`\\?\UNC\…`・`\\.\…`）。ドライブ文字つきの
/// 絶対のパス（`\\?\C:\…` を含む）と相対のパスは違う。文字だけで決める（ファイルには触らない）。
pub fn is_network_path(stored: &str) -> bool {
    let unified = stored.replace('\\', "/");
    let Some(rest) = unified.strip_prefix("//") else {
        return false;
    };
    // `\\?\C:\…` は、ドライブ文字つきのローカルのパス
    match rest.strip_prefix("?/") {
        Some(after) => {
            let mut chars = after.chars();
            !(chars.next().is_some_and(|c| c.is_ascii_alphabetic()) && chars.next() == Some(':'))
        }
        None => true,
    }
}

/// view.json の相対のパスの基準のフォルダー: .ylp のあるフォルダー。退避のフォルダーの中の退避（`yolu_io::backup_origin`）は、退避した元の
/// .ylp のあるフォルダー（退避は元のファイルの前の版で、パスは元のファイルから相対に書いてある）。
fn base_dir(ylp: &Path) -> PathBuf {
    // 退避の退避（前の版のアプリが、退避を開いて上書き保存して作った）も、元のファイルまでたどる
    let mut file = ylp.to_path_buf();
    while let Some(origin) = yolu_io::backup_origin(&file) {
        file = origin;
    }
    file.parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// view.json の文字列を、.ylp のあるフォルダ（退避なら元の .ylp のあるフォルダ）から見たモデルのファイルのパスにする。
pub fn resolve_model_path(stored: &str, ylp: &Path) -> PathBuf {
    let stored = Path::new(stored);
    if stored.is_absolute() {
        return stored.to_path_buf();
    }
    absolute(&base_dir(ylp).join(stored))
}

/// .ylp を開いたとき（`open_into` の終わり）: 参照があれば、モデルを読み始める。見つからない・読めない理由は、読み終えたときの知らせに
/// 出す（ここで返すのは、ファイルに触らずに分かる理由だけ）。
pub fn start(
    app: &mut AppState,
    ylp: &Path,
    stored: &str,
    base: &str,
    base_kind: crate::notice::Kind,
) -> Option<String> {
    let lang = app.lang;
    if is_network_path(stored) {
        // ネットワークのパスは、書かれたままを参照として残すだけで触らない（`resolve_model_path` で解くと、Windows では先頭の
        // `//` が「今のドライブの根」と読まれて別の場所の参照に変わり、保存し直すと壊れる）
        let path = PathBuf::from(stored);
        app.np.model_file = Some(path.clone());
        return Some(lang.pick(
            format!(
                "ネットワーク上のモデル{}は自動では読みません。",
                lang.quote(&file_name(&path))
            ),
            format!(
                "The model {} on the network is not read automatically.",
                lang.quote(&file_name(&path))
            ),
        ));
    }
    let path = resolve_model_path(stored, ylp);
    app.np.model_file = Some(path.clone());
    if app.model.as_ref().is_some_and(|m| m.is_link()) {
        // Unity のシーンのモデルが付いている間は、そちらを使う（参照だけ残す）
        return None;
    }
    // ファイルがあるかの確かめは別のスレッドで（遅いドライブや共有でも、画面を止めない）
    let (tx, rx) = channel();
    let probe = path.clone();
    std::thread::spawn(move || {
        let _ = tx.send(probe.is_file());
    });
    app.np.reopening = Some(Reopen {
        path,
        stage: Stage::Checking(rx),
        generation: app.np.generation,
        base: base.to_owned(),
        base_kind,
    });
    None
}

/// 毎フレーム: 確かめ終えたら読み始め、読み終えたモデルを入れる。
pub(super) fn poll(app: &mut AppState) {
    let Some(mut reopen) = app.np.reopening.take() else {
        return;
    };
    if reopen.generation != app.np.generation {
        return; // 別のプロジェクトになった
    }
    let lang = app.lang;
    let name = file_name(&reopen.path);
    let result = match &reopen.stage {
        Stage::Checking(rx) => match rx.try_recv() {
            Err(TryRecvError::Empty) => {
                app.np.reopening = Some(reopen);
                return;
            }
            Ok(true) => {
                let job = pose::prepare_fbx(
                    &mut app.view3d,
                    &reopen.path,
                    yolu_model::ModelLimits::default(),
                );
                reopen.stage = Stage::Loading(job);
                app.np.reopening = Some(reopen);
                return;
            }
            Ok(false) | Err(TryRecvError::Disconnected) => None,
        },
        Stage::Loading(job) => match job.poll() {
            Some(result) => Some(result),
            None => {
                app.np.reopening = Some(reopen);
                return;
            }
        },
    };
    use crate::notice::Kind;
    let (note_kind, note) = match result {
        None => (
            Kind::Warning,
            lang.pick(
                format!("モデル{}が見つかりません。", lang.quote(&name)),
                format!("The model {} was not found.", lang.quote(&name)),
            ),
        ),
        Some(Ok(prepared)) => {
            pose::install_prepared(&mut app.view3d, prepared);
            if let Some(session) = app.view3d.pose.session.as_ref() {
                app.model = Some(SceneModel::from_rig(&session.rig));
            }
            let report = app.bind_model_only();
            let mut text = lang.pick(format!("モデル: {name}。"), format!("Model: {name}."));
            let mut kind = Kind::Info;
            if !report.unmatched.is_empty() {
                kind = Kind::Warning;
                text += &lang.pick(
                    format!(" モデルに無いセット {}。", report.unmatched.len()),
                    format!(" Not in the model: {}.", report.unmatched.len()),
                );
            }
            // ファイルのポーズ（pose.json）を戻す（合わない項目は飛ばして理由をポーズの欄に残す。取り消しの段にも変更の印にもしない）
            if let Some(note) = crate::view3d::pose::stored::restore_from_project(app) {
                kind = Kind::Warning;
                text += &format!(" {note}");
            }
            // 焼いたマップは、読み終えた（ポーズを戻した）このモデルで照合し直す
            app.expect_reopen_check();
            (kind, text)
        }
        Some(Err(e)) => (
            Kind::Error,
            lang.with_reason(
                lang.pick(
                    format!("モデル{}を読み込めません", lang.quote(&name)),
                    format!("Cannot load the model {}", lang.quote(&name)),
                ),
                lang.view_error(&e),
            ),
        ),
    };
    // 注意・失敗はログの行（1 行に切る）に残る。長い開いた知らせの後ろに続けると、モデルの理由が切れて読めないので単独の知らせにする
    // （開いた知らせは、開いたときに出してある）。残らない知らせ同士は、1 つの文にまとめて状態の帯に出す
    if note_kind.is_logged() || reopen.base_kind.is_logged() || reopen.base.is_empty() {
        app.notify(note_kind, crate::notice::Source::Open, note);
    } else {
        app.notify(
            reopen.base_kind.worse(note_kind),
            crate::notice::Source::Open,
            format!("{} {note}", reopen.base),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stored_path_is_relative_to_the_project_folder() {
        let ylp = Path::new("/work/proj/art/p.ylp");
        for (model, stored) in [
            ("/work/proj/art/body.fbx", "body.fbx"),
            ("/work/proj/art/models/body.fbx", "models/body.fbx"),
            ("/work/proj/models/body.fbx", "../models/body.fbx"),
            ("/work/other/x/body.fbx", "../../other/x/body.fbx"),
            ("/work/proj/art/./models/../body.fbx", "body.fbx"),
        ] {
            assert_eq!(
                relative_model_path(Path::new(model), ylp),
                stored,
                "{model}"
            );
            let back = resolve_model_path(stored, ylp);
            assert_eq!(back, absolute(Path::new(model)), "{stored}");
        }
    }

    /// 退避のフォルダーの中の退避は、元の .ylp のあるフォルダーから解く（本体と同じ view.json の文字列が同じモデルを指す）。保存で書き直す
    /// 相対のパスも同じ基準。名前が退避に合わないファイルは、置いてあるフォルダーが基準のまま。
    #[test]
    fn a_backup_resolves_and_writes_the_model_path_from_the_file_it_backs_up() {
        let main = Path::new("/work/proj/art/p.ylp");
        let backup = Path::new("/work/proj/art/p.ylp-backups~/p-20260101T000000000Z.ylp");
        let model = Path::new("/work/proj/models/body.fbx");
        for stored in ["../models/body.fbx", "x/../../models/body.fbx"] {
            assert_eq!(
                resolve_model_path(stored, backup),
                resolve_model_path(stored, main),
                "{stored}"
            );
            assert_eq!(resolve_model_path(stored, backup), absolute(model));
        }
        assert_eq!(relative_model_path(model, backup), "../models/body.fbx");
        assert_eq!(
            relative_model_path(model, backup),
            relative_model_path(model, main)
        );
        // 名前が退避に合わない・退避のフォルダーでない所のファイルは、置いてあるフォルダーが基準
        for other in [
            "/work/proj/art/p.ylp-backups~/keep.ylp",
            "/work/proj/art/backups/p-20260101T000000000Z.ylp",
        ] {
            let other = Path::new(other);
            assert_eq!(
                resolve_model_path("m.fbx", other),
                absolute(&other.with_file_name("m.fbx")),
                "{other:?}"
            );
        }
    }

    /// 退避を開いて上書き保存した前の版のアプリが作った、退避の中の退避（`p.ylp-backups~/p-<時刻>.ylp-backups~/p-<時刻>-<時刻>.ylp`）も、
    /// 元のファイルまでたどって同じ基準にする（読むときも、保存で書き直す相対のパスも）。
    #[test]
    fn a_backup_of_a_backup_is_traced_back_to_the_original_file() {
        let main = Path::new("/work/proj/art/p.ylp");
        let first = "/work/proj/art/p.ylp-backups~/p-20260101T000000000Z.ylp";
        let second = format!("{first}-backups~/p-20260101T000000000Z-20260102T000000000Z.ylp");
        let third = format!(
            "{second}-backups~/p-20260101T000000000Z-20260102T000000000Z-20260103T000000000Z.ylp"
        );
        let model = Path::new("/work/proj/models/body.fbx");
        for nested in [first, second.as_str(), third.as_str()] {
            let nested = Path::new(nested);
            assert_eq!(
                resolve_model_path("../models/body.fbx", nested),
                resolve_model_path("../models/body.fbx", main),
                "{nested:?}"
            );
            assert_eq!(
                relative_model_path(model, nested),
                relative_model_path(model, main),
                "{nested:?}"
            );
        }
    }

    #[test]
    fn network_paths_are_told_by_their_text_alone() {
        for stored in [
            "//nas/share/x.fbx",
            "\\\\nas\\share\\x.fbx",
            "\\\\?\\UNC\\nas\\share\\x.fbx",
            "//?/UNC/nas/share/x.fbx",
            "\\\\.\\pipe\\x",
            "//./pipe/x",
            "\\\\nas@80\\x.fbx",
            "//localhost/c$/x.fbx",
        ] {
            assert!(is_network_path(stored), "{stored}");
        }
        for stored in [
            "",
            "x.fbx",
            "models/x.fbx",
            "../models/x.fbx",
            "C:/models/x.fbx",
            "C:\\models\\x.fbx",
            "\\\\?\\C:\\models\\x.fbx",
            "//?/c:/models/x.fbx",
            "/home/user/x.fbx",
            "/x.fbx",
        ] {
            assert!(!is_network_path(stored), "{stored}");
        }
    }

    /// 相対のパスは、.ylp のあるフォルダの中だけを指す（`..` でも、別のホストへは出ない）。
    #[test]
    fn a_relative_path_never_leaves_the_project_folder_for_another_host() {
        let ylp = Path::new("/work/proj/p.ylp");
        for stored in [
            "../../../../../nas/share/x.fbx",
            "models/../../x.fbx",
            "./x.fbx",
        ] {
            let resolved = resolve_model_path(stored, ylp);
            assert!(
                resolved.is_absolute() && !is_network_path(&resolved.to_string_lossy()),
                "{resolved:?}"
            );
        }
    }

    /// Windows 以外でも、ネットワークのパスは別の相対のパスに変わらず、書かれたまま（'/' 区切り）残る。
    #[cfg(not(windows))]
    #[test]
    fn a_network_model_is_kept_as_written_off_windows() {
        let ylp = Path::new("/work/proj/p.ylp");
        for model in [
            "//nas/share/models/body.fbx",
            "\\\\nas\\share\\models\\body.fbx",
            "//nas/body.fbx",
        ] {
            let stored = relative_model_path(Path::new(model), ylp);
            assert_eq!(stored, model.replace('\\', "/"), "{model}");
            assert!(is_network_path(&stored), "{stored}");
        }
        // ローカルのパスは今までどおり相対にする
        assert_eq!(
            relative_model_path(Path::new("/work/models/body.fbx"), ylp),
            "../models/body.fbx"
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_network_model_is_stored_as_a_unc_path_and_a_model_on_the_same_share_stays_relative() {
        let ylp = Path::new("C:\\work\\p.ylp");
        let stored = relative_model_path(Path::new("\\\\nas\\share\\models\\body.fbx"), ylp);
        assert_eq!(stored, "//nas/share/models/body.fbx");
        assert!(is_network_path(&stored));
        // .ylp も同じ共有にあれば、相対で残す
        let on_share = Path::new("\\\\nas\\share\\proj\\p.ylp");
        let stored = relative_model_path(Path::new("\\\\nas\\share\\models\\body.fbx"), on_share);
        assert_eq!(stored, "../models/body.fbx");
        assert!(!is_network_path(&stored));
    }

    #[test]
    fn an_absolute_stored_path_is_used_as_it_is() {
        let ylp = Path::new("/work/proj/p.ylp");
        let abs = if cfg!(windows) {
            "C:/models/body.fbx"
        } else {
            "/models/body.fbx"
        };
        assert_eq!(resolve_model_path(abs, ylp), PathBuf::from(abs));
    }

    #[cfg(windows)]
    #[test]
    fn a_model_on_another_drive_is_stored_as_an_absolute_path() {
        let stored = relative_model_path(
            Path::new("D:\\models\\body.fbx"),
            Path::new("C:\\work\\p.ylp"),
        );
        assert_eq!(stored, "D:/models/body.fbx");
        assert_eq!(
            resolve_model_path(&stored, Path::new("C:\\work\\p.ylp")),
            PathBuf::from("D:/models/body.fbx")
        );
    }
}
