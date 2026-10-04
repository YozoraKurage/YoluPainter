//! プロジェクトのモデルのファイル（FBX）を .ylp の view.json に残し、開くと読み直す。
//!
//! - 残す形は .ylp からの相対のパス（'/' 区切り。同じ場所から辿れなければ絶対のパス）。プロジェクトとモデルを同じフォルダの
//!   組で動かしても、参照は切れない。形式も正本の版も変えない（`yolu_io::Project::with_view_model`。view.json の任意の項目）。
//! - 開くとき: ファイルがあれば別のスレッドで読み（終わるまで描ける。取消は仕事の札）、読み終えたら 3D ビューに入れ、セットを
//!   鍵で結び付ける（モデルのマテリアルにセットを増やさない）。ファイルが無くても参照は残す（保存で失わず、構成の「読み直す」で
//!   見つかったときに読める）。
//! - Live Link のモデル（Unity のシーンのもの）が付いているときは、モデルのファイルは読まない（参照だけ残す）。

use std::path::{Component, Path, PathBuf};

use crate::model::SceneModel;
use crate::state::AppState;
use crate::view3d::pose::{self, PrepareJob};

/// .ylp を開いて、モデルを読んでいる最中。
pub struct Reopen {
    pub path: PathBuf,
    job: PrepareJob,
    /// 始めたときのプロジェクトの世代（変わったら結果を捨てる）。
    generation: u64,
    /// 開いたときの知らせ（読み終えたら、その後ろにモデルの知らせを足す）。
    base: String,
}

impl std::fmt::Debug for Reopen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reopen").field("path", &self.path).finish()
    }
}

impl Reopen {
    /// 取り消す（読み込みは止まり、結果は捨てる）。
    pub fn cancel(&self) {
        self.job.cancel();
    }
    /// 読んでいるファイルの名前。
    pub fn file_name(&self) -> String {
        file_name(&self.path)
    }
}

impl Drop for Reopen {
    fn drop(&mut self) {
        self.job.cancel();
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
    let model = absolute(model);
    let base = absolute(ylp.parent().unwrap_or_else(|| Path::new(".")));
    let m: Vec<Component> = model.components().collect();
    let b: Vec<Component> = base.components().collect();
    let common = m.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let slash = |p: &Path| p.to_string_lossy().replace('\\', "/");
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

/// view.json の文字列を、.ylp のあるフォルダから見たモデルのファイルのパスにする。
pub fn resolve_model_path(stored: &str, ylp: &Path) -> PathBuf {
    let stored = Path::new(stored);
    if stored.is_absolute() {
        return stored.to_path_buf();
    }
    absolute(&ylp.parent().unwrap_or_else(|| Path::new(".")).join(stored))
}

/// .ylp を開いたとき（`open_into` の終わり）: 参照があれば、モデルを読み始める。見つからない・読めない理由を短く返す。
pub fn start(app: &mut AppState, ylp: &Path, stored: &str, base: &str) -> Option<String> {
    let lang = app.lang;
    let path = resolve_model_path(stored, ylp);
    app.np.model_file = Some(path.clone());
    if !path.is_file() {
        return Some(lang.pick(
            format!("モデルが見つかりません: {}。", file_name(&path)),
            format!("Model not found: {}.", file_name(&path)),
        ));
    }
    if app.model.as_ref().is_some_and(|m| m.is_link()) {
        // Unity のシーンのモデルが付いている間は、そちらを使う（参照だけ残す）
        return None;
    }
    let job = pose::prepare_fbx(&mut app.view3d, &path, yolu_model::ModelLimits::default());
    app.np.reopening = Some(Reopen {
        path,
        job,
        generation: app.np.generation,
        base: base.to_owned(),
    });
    None
}

/// 毎フレーム: 読み終えたモデルを入れる。
pub(super) fn poll(app: &mut AppState) {
    let Some(reopen) = app.np.reopening.take() else {
        return;
    };
    let Some(result) = reopen.job.poll() else {
        app.np.reopening = Some(reopen);
        return;
    };
    if reopen.generation != app.np.generation {
        return; // 別のプロジェクトになった
    }
    let lang = app.lang;
    let name = file_name(&reopen.path);
    let note = match result {
        Ok(prepared) => {
            pose::install_prepared(&mut app.view3d, prepared);
            if let Some(session) = app.view3d.pose.session.as_ref() {
                app.model = Some(SceneModel::from_rig(&session.rig));
            }
            let report = app.bind_model_only();
            let mut text = lang.pick(format!("モデル: {name}。"), format!("Model: {name}."));
            if !report.unmatched.is_empty() {
                text += &lang.pick(
                    format!(" モデルに無いセット {}。", report.unmatched.len()),
                    format!(" Not in the model: {}.", report.unmatched.len()),
                );
            }
            text
        }
        Err(e) => format!("{name}: {}", lang.view_error(&e)),
    };
    app.message = if reopen.base.is_empty() {
        note
    } else {
        format!("{} {note}", reopen.base)
    };
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
