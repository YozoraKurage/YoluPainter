//! ファイルを選ぶウィンドウの始まりの場所（`dialog::places`）。始まりの場所の決め方は純粋な関数（ソースの単体試験）で見ているので、ここは文書の状態（保存した
//! プロジェクト・退避を開いた文書・前に使った場所の覚え）につないだ決め方と、覚えた場所が設定のフォルダに残って読み直せることを見る。
//! OS のウィンドウは開かない。
use std::path::{Path, PathBuf};

use yolu_app::dialog::places::{Place, Places, Rule};
use yolu_app::dialog::start_folder_with;
use yolu_app::state::{Action, AppState};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-places-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn folder(root: &Path, name: &str) -> PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// プロジェクトを `dir` の中に保存した文書。
fn saved_in(dir: &Path) -> AppState {
    let mut s = AppState::new(32, 32);
    s.apply(Action::SaveProjectAs(dir.join("doc.ylp")));
    assert!(dir.join("doc.ylp").exists(), "{}", s.message);
    s
}

#[test]
fn opening_starts_where_that_kind_was_last_used_and_otherwise_in_the_documents_folder_of_the_document(
) {
    let root = temp("open");
    let (doc_dir, models, documents) = (
        folder(&root, "doc"),
        folder(&root, "models"),
        folder(&root, "documents"),
    );
    let mut s = saved_in(&doc_dir);
    assert_eq!(s.document_folder(), Some(doc_dir.clone()));
    // 前に使った場所が無い種類は、今の文書のフォルダ
    for place in [Place::Model, Place::PsdImport, Place::Brush] {
        assert_eq!(
            start_folder_with(&s, place, Rule::Open, Some(&documents)),
            Some(doc_dir.clone()),
            "{place:?}"
        );
    }
    // 選んだファイルのあるフォルダを覚える。その種類だけが前の場所から始まる
    s.note_file_chosen(Place::Model, &models.join("body.fbx"));
    assert_eq!(
        start_folder_with(&s, Place::Model, Rule::Open, Some(&documents)),
        Some(models.clone())
    );
    assert_eq!(
        start_folder_with(&s, Place::PsdImport, Rule::Open, Some(&documents)),
        Some(doc_dir.clone()),
        "ほかの種類には移らない"
    );
    // 選んだフォルダ（書き出し先など）はそのフォルダ自身を覚える
    s.note_folder_chosen(Place::ImageExport, &documents);
    assert_eq!(
        start_folder_with(&s, Place::ImageExport, Rule::Open, None),
        Some(documents.clone())
    );
    // 無くなった場所は使わない（文書のフォルダへ）
    std::fs::remove_dir_all(&models).unwrap();
    assert_eq!(
        start_folder_with(&s, Place::Model, Rule::Open, Some(&documents)),
        Some(doc_dir.clone())
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_document_that_was_never_saved_starts_in_the_os_documents_folder_and_never_in_a_missing_one() {
    let root = temp("fresh");
    let documents = folder(&root, "documents");
    let s = AppState::new(32, 32);
    assert_eq!(s.document_folder(), None);
    for rule in [Rule::Open, Rule::Save] {
        assert_eq!(
            start_folder_with(&s, Place::Project, rule, Some(&documents)),
            Some(documents.clone()),
            "{rule:?}"
        );
        // OS の書類のフォルダも無ければ、OS に任せる（「/」を自分で選ばない）
        assert_eq!(
            start_folder_with(&s, Place::Project, rule, Some(&root.join("missing"))),
            None
        );
        assert_eq!(start_folder_with(&s, Place::Project, rule, None), None);
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn saving_starts_in_the_folder_of_the_document_then_the_last_place_and_the_saved_origin_comes_first(
) {
    let root = temp("save");
    let (doc_dir, last, origin, documents) = (
        folder(&root, "doc"),
        folder(&root, "last"),
        folder(&root, "origin"),
        folder(&root, "documents"),
    );
    // 保存した文書: 前に使った場所があっても、文書のフォルダが先
    let mut s = saved_in(&doc_dir);
    s.note_file_chosen(Place::Project, &last.join("other.ylp"));
    assert_eq!(
        start_folder_with(&s, Place::Project, Rule::Save, Some(&documents)),
        Some(doc_dir.clone())
    );
    // 開くほうは、前に使った場所が先
    assert_eq!(
        start_folder_with(&s, Place::Project, Rule::Open, Some(&documents)),
        Some(last.clone())
    );
    // 退避を開いた文書の元のフォルダ（`save_folder`）は、保存で一番先
    s.save_folder = Some(origin.clone());
    assert_eq!(
        start_folder_with(&s, Place::Project, Rule::Save, Some(&documents)),
        Some(origin.clone())
    );
    assert_eq!(
        start_folder_with(&s, Place::Project, Rule::Open, Some(&documents)),
        Some(last.clone()),
        "開くときは使わない"
    );
    // まだ保存していない文書は、前に使った場所
    let mut fresh = AppState::new(32, 32);
    fresh.note_file_chosen(Place::Project, &last.join("other.ylp"));
    assert_eq!(
        start_folder_with(&fresh, Place::Project, Rule::Save, Some(&documents)),
        Some(last)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn remembered_places_stay_in_the_settings_folder_and_are_read_again_and_a_test_state_never_writes()
{
    let root = temp("persist");
    let file = root.join("config").join("places.conf");
    let kept = folder(&root, "kept");
    let mut s = AppState::new(32, 32);
    s.places = Places::load(Some(file.clone()));
    s.note_file_chosen(Place::Font, &kept.join("a.ttf"));
    s.note_folder_chosen(Place::Brush, &kept);
    assert!(file.exists());
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("font=") && text.contains("brush="), "{text}");
    // 開き直し（別の AppState）
    let mut t = AppState::new(32, 32);
    t.places = Places::load(Some(file.clone()));
    assert_eq!(t.places.get(Place::Font), Some(kept.as_path()));
    assert_eq!(
        start_folder_with(&t, Place::Brush, Rule::Open, None),
        Some(kept.clone())
    );
    // 新しい AppState は、ファイルを持たない（覚えても、設定のフォルダへ書かない）
    let mut plain = AppState::new(32, 32);
    plain.note_folder_chosen(Place::Keys, &kept);
    assert_eq!(plain.places.get(Place::Keys), Some(kept.as_path()));
    let real = Places::default_path();
    if let Some(real) = real {
        if let Ok(text) = std::fs::read_to_string(real) {
            assert!(!text.contains(&format!("keys={}", kept.display())));
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
