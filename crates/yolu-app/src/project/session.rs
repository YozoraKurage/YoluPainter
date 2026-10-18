//! 開いているプロジェクトの状態（`Session`）。今の文書・テクスチャセット・モデル・ファイルとの結び付き・名前・変更の印と世代を持つ。
//!
//! 文書の入れ替え（`install`）・世代（`doc_epoch`・`project_epoch`）・保存していない変更の印（`modified`）は外から直に書けず、
//! ここのメソッドだけが変える。ほかの欄は `pub`。

use crate::engine::{Document, LayerId, LayerSetId, Stroke};
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
    /// 今のレイヤーセット（選んでいるレイヤーが無いときに効く。選んでいるレイヤーがあれば、そのレイヤーのレイヤーセットが今のレイヤーセット。
    /// 読むのは `AppState::layer_set`。保存しない。None と文書に無い ID は最初のレイヤーセット）。
    pub layer_set: Option<LayerSetId>,
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
    /// 文書の外の変更（セットの名前・並び・書き出しの設定・アセット・ポーズなど）と、文書の履歴に収まらない編集があったか。消えるのは保存したときだけ。
    modified: bool,
    /// 文書の中身を変えうる編集（ストローク・Undo・レイヤーの操作）をしたか（[`Session::mark_content_modified`]）。立っていても、文書が保存した所と
    /// 同じ中身（`Document::is_clean`。画素の変わらないストローク・取り消したストローク・Undo で保存した所へ戻った）なら、変更の印にならない。
    content_marked: bool,
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
        mut doc: Document,
        selected_layer: Option<LayerId>,
        sets: TextureSets,
        project_name: String,
    ) -> Session {
        // できたての文書（最初のレイヤーを足した後）は、保存した所にいる
        doc.mark_saved();
        Session {
            doc,
            doc_epoch: 0,
            stroke: None,
            selected_layer,
            layer_set: None,
            sets,
            model: None,
            project: None,
            project_name,
            save_folder: None,
            project_epoch: 0,
            modified: false,
            content_marked: false,
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

    /// 保存していない変更があるか（メニューバーの右の「•」。新規・開くの前に捨ててよいかを聞く）。文書の外の変更の印があるか、文書の編集をして
    /// 今の文書が保存した所と違うとき（Undo・取り消しで保存した所へ戻れば違わない）。
    pub fn is_modified(&self) -> bool {
        self.modified || (self.content_marked && !self.doc.is_clean())
    }

    /// 今の文書を `doc` に替えて、文書の世代を進める。返すのは前の文書。画面の途中の状態を新しい文書に合わせるのは
    /// `AppState::install_document`。
    pub(crate) fn install(&mut self, mut doc: Document) -> Document {
        // 入れる文書の今の中身を保存した所とする（読み込んだ直後の整え・しまっていた文書。保存していない変更は、外れるときに文書の外の印へ
        // 移してある）
        doc.mark_saved();
        let previous = std::mem::replace(&mut self.doc, doc);
        // 外れる文書の保存していない変更は、これからは文書の外の印が持つ（しまった文書は、次に出てきたときこの目印を持たない）
        if self.content_marked && !previous.is_clean() {
            self.mark_modified();
        }
        self.content_marked = false;
        self.doc_epoch = self.doc_epoch.wrapping_add(1);
        previous
    }

    /// 文書の世代だけを進める（テストが、同じ `Document` のまま中身を替えたことにして、キャンバスの表示に前の合成を捨てさせる）。
    /// アプリの中で世代を進めるのは [`Session::install`] だけ。
    #[cfg(test)]
    pub(crate) fn advance_doc_epoch(&mut self) {
        self.doc_epoch = self.doc_epoch.wrapping_add(1);
    }

    /// プロジェクトを替えたことを記す（世代を進める）。
    pub fn bump_project_epoch(&mut self) {
        self.project_epoch += 1;
    }

    /// 文書の中身を変えうる編集をした印を立てる（ストロークを始めた・Undo・Redo・レイヤーの操作）。実際に変わったかは文書が答える
    /// （[`Session::is_modified`]）ので、画素を変えずに終えた・取り消した・Undo で保存した所へ戻したときは変更の印にならない。
    pub fn mark_content_modified(&mut self) {
        self.content_marked = true;
    }

    /// 今の文書のリビジョンが `before`（編集の前に控えた `doc.revision()`）と違えば、文書の中身を変えうる編集をした印を立てる。
    pub fn mark_content_modified_if_changed(&mut self, before: u64) {
        if self.doc.revision() != before {
            self.mark_content_modified();
        }
    }

    /// 保存していない変更がある印を立てる（文書の外の変更と、文書の履歴に収まらない編集。Undo で保存した所へ戻っても消えない）。
    pub fn mark_modified(&mut self) {
        self.modified = true;
    }

    /// 今の文書のリビジョンが `before`（編集の前に控えた `doc.revision()`）と違えば、保存していない変更の印を立てる（Undo で戻っても消えない印）。
    pub fn mark_modified_if_changed(&mut self, before: u64) {
        if self.doc.revision() != before {
            self.mark_modified();
        }
    }

    /// 保存していない変更の印を下ろす（保存した・開いた・プロジェクトを替えた）。今の文書もしまった文書も、今の中身を保存した所として覚える。
    pub fn clear_modified(&mut self) {
        self.modified = false;
        self.content_marked = false;
        self.doc.mark_saved();
        self.sets.mark_stashed_saved();
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
        let sample = include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../yolu-io/tests/fixtures/format1.ylp"
        ));
        s.project = Some(ProjectFile {
            path: std::path::PathBuf::from("/tmp/x/作品.ylp"),
            target: None,
            original: std::sync::Arc::new(yolu_io::Project::read(sample).unwrap()),
        });
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
