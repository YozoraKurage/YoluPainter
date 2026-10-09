//! 文書（docs/GUIDE_KEYS.md と英語の同じ文書）の「既定の割り当て」の表が、アプリの既定の表から作った物と同じか。表を変えたら
//! `YOLU_UPDATE_GUIDE_KEYS=1 cargo test -p yolu-app --test headless guide_keys` で書き直す。
use std::path::Path;

use yolu_app::lang::Lang;
use yolu_app::shortcuts::guide;

#[test]
fn the_default_key_tables_in_the_guides_are_made_from_the_keymap() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs");
    for (file, lang) in [("GUIDE_KEYS.md", Lang::Ja), ("en/GUIDE_KEYS.md", Lang::En)] {
        let path = root.join(file);
        let doc = std::fs::read_to_string(&path)
            .unwrap()
            .replace("\r\n", "\n");
        let made = guide::replaced(&doc, lang).expect("表を入れる印");
        if std::env::var_os("YOLU_UPDATE_GUIDE_KEYS").is_some() {
            std::fs::write(&path, &made).unwrap();
            continue;
        }
        assert!(
            doc == made,
            "{file} の既定の割り当ての表が、アプリの表と違う（YOLU_UPDATE_GUIDE_KEYS=1 で書き直す）"
        );
    }
}
