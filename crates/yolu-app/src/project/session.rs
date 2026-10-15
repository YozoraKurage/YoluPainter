//! 開いているプロジェクトの状態（`Session`）。今の文書・テクスチャセット・モデル・ファイルとの結び付き・名前・変更の印と世代を持つ。
//!
//! 文書の入れ替え（`install`）・世代（`doc_epoch`・`project_epoch`）・保存していない変更の印（`modified`）は外から直に書けず、
//! ここのメソッドだけが変える。ほかの欄は `pub`。

use crate::engine::{Document, LayerId, Stroke};
use crate::model::SceneModel;
use crate::project::ProjectFile;
use crate::sets::TextureSets;

/// 開いているプロジェクトの状態（`AppState::session`）。
pub struct Session {
    pub doc: Document,
    /// 文書を別のものに替えた回数（開く・新しく作る・PSD を読み込む・テクスチャセットを切り替える）。キャンバスの表示は、文書 ID が
    /// 同じでも（.ylp や PSD を読み直すと保存した ID が戻る）これが変わったら、前の文書の合成を捨てて作り直す。文書を丸ごと
    /// 置き換える口は [`Session::install`] を通す。
    doc_epoch: u64,
    /// 描いているストローク（core の `Stroke`。文書を借りないのでフレームをまたいで持つ）。
    pub stroke: Option<Stroke>,
    pub selected_layer: Option<LayerId>,
    /// テクスチャセット（今のセットの文書は `doc`）。
    pub sets: TextureSets,
    /// 読み込んだモデル（Live Link で受けたもの。3D ビューが読む）。
    pub model: Option<SceneModel>,
    /// 開いた .ylp（保存先と、保存で残す元の中身）。
    pub project: Option<ProjectFile>,
    pub project_name: String,
    /// 次に名前を付けて保存のウィンドウを開く場所（退避を開いたときの、元の .ylp のフォルダー。保存したら外す。無ければ OS の既定）。
    pub save_folder: Option<std::path::PathBuf>,
    /// プロジェクトを替えた回数（新規・開く・作る。`np_project_replaced` が進める）。Live Link の裏の仕事が、始めた時のプロジェクトへだけ入るための目印。
    project_epoch: u64,
    /// 開いた・保存した後に変えたか（メニューバーの右の「•」。新規・開くの前に捨ててよいかを聞く）。
    modified: bool,
    /// 新規プロジェクトのウィンドウで、利用者が解像度を選んで作ったプロジェクトか（Live Link の元の絵が、最初のセットを元の絵の大きさで作り直してよいかを
    /// 決める。選んだ大きさは元の絵で上書きしない）。起動時の既定・ファイルの「新規」・開いたプロジェクトでは false。
    pub resolution_chosen: bool,
    /// 直前の保存で書き直したテクスチャセット（正本）の数。画面には出さない（テストが、変えていないセットを書き直さないことを確かめる）。
    #[doc(hidden)]
    pub rewritten_sets: usize,
}

impl Session {
    /// 文書 `doc`（選んだレイヤーは `selected_layer`）とテクスチャセット `sets` の、まだファイルの無いプロジェクト `project_name`。
    pub(crate) fn new(
        doc: Document,
        selected_layer: Option<LayerId>,
        sets: TextureSets,
        project_name: String,
    ) -> Session {
        Session {
            doc,
            doc_epoch: 0,
            stroke: None,
            selected_layer,
            sets,
            model: None,
            project: None,
            project_name,
            save_folder: None,
            project_epoch: 0,
            modified: false,
            resolution_chosen: false,
            rewritten_sets: 0,
        }
    }

    /// 文書を入れ替えた回数（[`Session::install`] が進める）。
    pub fn doc_epoch(&self) -> u64 {
        self.doc_epoch
    }

    /// プロジェクトを替えた回数（[`Session::bump_project_epoch`] が進める）。
    pub fn project_epoch(&self) -> u64 {
        self.project_epoch
    }

    /// 保存していない変更があるか。
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// 今の文書を `doc` に替えて、文書の世代を進める。返すのは前の文書。画面の途中の状態を新しい文書に合わせるのは
    /// `AppState::install_document`。
    pub(crate) fn install(&mut self, doc: Document) -> Document {
        let previous = std::mem::replace(&mut self.doc, doc);
        self.advance_doc_epoch();
        previous
    }

    /// 文書の世代だけを進める（同じ `Document` の中身を丸ごと替えたときに、キャンバスの表示に前の合成を捨てさせる）。
    pub(crate) fn advance_doc_epoch(&mut self) {
        self.doc_epoch = self.doc_epoch.wrapping_add(1);
    }

    /// プロジェクトを替えたことを記す（世代を進める）。
    pub fn bump_project_epoch(&mut self) {
        self.project_epoch += 1;
    }

    /// 保存していない変更がある印を立てる。
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    /// 今の文書のリビジョンが `before`（編集の前に控えた `doc.revision()`）と違えば、保存していない変更の印を立てる。
    pub fn mark_modified_if_changed(&mut self, before: u64) {
        if self.doc.revision() != before {
            self.mark_modified();
        }
    }

    /// 保存していない変更の印を下ろす（保存した・開いた・プロジェクトを替えた）。
    pub fn clear_modified(&mut self) {
        self.modified = false;
    }

    /// プロジェクトの名乗りを初期化する（ファイルとの結び付きを空に、名前を `name` に、保存先のフォルダーを無しに、変更の印を下ろす）。
    pub(crate) fn reset_identity(&mut self, name: String) {
        self.project = None;
        self.project_name = name;
        self.save_folder = None;
        self.clear_modified();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        let (doc, first) = crate::state::blank_document(64, 64);
        let sets = TextureSets::first_in(&doc, crate::lang::Lang::Ja);
        Session::new(doc, first, sets, "名称未設定".into())
    }

    #[test]
    fn install_advances_doc_epoch_and_returns_previous_document() {
        let mut s = session();
        let before = s.doc_epoch();
        let old_id = s.doc.id();
        let (doc, _) = crate::state::blank_document(32, 32);
        let new_id = doc.id();
        let previous = s.install(doc);
        assert_eq!(previous.id(), old_id);
        assert_eq!(s.doc.id(), new_id);
        assert_eq!(s.doc_epoch(), before + 1);
    }

    #[test]
    fn mark_modified_if_changed_leaves_mark_when_revision_is_same() {
        let mut s = session();
        let before = s.doc.revision();
        s.mark_modified_if_changed(before);
        assert!(!s.is_modified());
        s.doc.add_layer("追加").unwrap();
        s.mark_modified_if_changed(before);
        assert!(s.is_modified());
        s.clear_modified();
        assert!(!s.is_modified());
    }

    #[test]
    fn reset_identity_clears_name_folder_file_and_mark() {
        let mut s = session();
        s.project_name = "作品".into();
        s.save_folder = Some(std::path::PathBuf::from("/tmp/x"));
        s.mark_modified();
        s.reset_identity("新しい".into());
        assert!(s.project.is_none());
        assert_eq!(s.project_name, "新しい");
        assert!(s.save_folder.is_none());
        assert!(!s.is_modified());
    }

    #[test]
    fn bump_project_epoch_advances_by_one() {
        let mut s = session();
        assert_eq!(s.project_epoch(), 0);
        s.bump_project_epoch();
        s.bump_project_epoch();
        assert_eq!(s.project_epoch(), 2);
    }
}
