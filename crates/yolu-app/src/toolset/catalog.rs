//! 「＋」のウィンドウ（ブラシを追加）の中身: 種類ごと（組み込み・同梱の Krita・Photoshop・CLIP STUDIO・そのほかの取り込み・自分のブラシ）の
//! 一覧・名前の検索・選び。選んだ物は今のグループの後ろへ置く（`BrushAction::AddFrom`。並びにある物と同梱の Krita は写しのファイルを作る）。
//! 利用者のブラシのファイル（取り込んだ物・自分で作った物・写し）は、並びから外した物もここに出る。画面は `panels::brush_catalog`。

use egui::Vec2;

use crate::brushes::{builtin, store, BrushKey, Entry};
use crate::lang::Lang;
use crate::state::AppState;

/// 「＋」のウィンドウの 1 つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CatalogItem {
    /// 組み込み（`builtin` の id）。
    Builtin(&'static str),
    /// 同梱の Krita 4 のブラシ（同梱の並びの添字）。
    Krita(usize),
    /// 利用者のブラシのファイル（番号）。
    User(u32),
}

impl CatalogItem {
    /// 一覧のブラシの印（同梱の Krita はまだファイルが無いので None）。
    pub fn key(self) -> Option<BrushKey> {
        match self {
            CatalogItem::Builtin(id) => Some(BrushKey::Builtin(id)),
            CatalogItem::User(id) => Some(BrushKey::User(id)),
            CatalogItem::Krita(_) => None,
        }
    }
}

/// ウィンドウの左の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Builtin,
    Krita,
    /// Photoshop の ABR と模様から取り込んだ物（とその写し）。
    Photoshop,
    ClipStudio,
    /// GIMP・PNG ほかから取り込んだ物。
    OtherImports,
    /// 自分で作った物・組み込みや同梱の Krita の写し。
    Mine,
}

impl Kind {
    pub const ALL: [Kind; 6] = [
        Kind::Builtin,
        Kind::Krita,
        Kind::Photoshop,
        Kind::ClipStudio,
        Kind::OtherImports,
        Kind::Mine,
    ];

    pub fn name(self, lang: Lang) -> &'static str {
        match self {
            Kind::Builtin => lang.pick("組み込み", "Built-in"),
            Kind::Krita => "Krita",
            Kind::Photoshop => "Photoshop",
            Kind::ClipStudio => "CLIP STUDIO",
            Kind::OtherImports => lang.pick("そのほかの取り込み", "Other Imports"),
            Kind::Mine => lang.pick("自分のブラシ", "My Brushes"),
        }
    }

    pub fn icon(self) -> &'static str {
        match self {
            // 組み込みと同梱の Krita はどちらもアプリに入っているブラシ。取り込みの矢印は取り込んだ物だけ
            Kind::Builtin | Kind::Krita => "paint_brush",
            Kind::Photoshop | Kind::ClipStudio | Kind::OtherImports => "import",
            Kind::Mine => "stylus",
        }
    }
}

/// 利用者のブラシのファイルの種類（取り込んだ出どころから）。
pub fn kind_of(entry: &Entry) -> Kind {
    match &entry.import {
        None => Kind::Mine,
        Some(meta) if meta.source.starts_with("Photoshop") => Kind::Photoshop,
        Some(meta) if meta.source.starts_with("CLIP STUDIO") => Kind::ClipStudio,
        // 同梱の Krita から置いた写しは、自分のブラシ
        Some(meta) if meta.source.starts_with("Krita") => Kind::Mine,
        Some(_) => Kind::OtherImports,
    }
}

/// ウィンドウの状態（開いているか・種類・検索・選んだ物・位置・スクロール）。
pub struct Catalog {
    pub open: bool,
    pub kind: Kind,
    pub search: String,
    /// 選んだ物（選んだ順）。
    pub selected: Vec<CatalogItem>,
    pub scroll: f32,
    pub content: f32,
    /// 見出しのドラッグで動いた量（画面の真ん中からの）。
    pub offset: Vec2,
    pub placed: bool,
    /// 右クリックのメニューの対象。
    pub context: Option<CatalogItem>,
    /// 消してよいかを確かめているファイル。
    pub pending_delete: Option<BrushKey>,
}

impl Default for Catalog {
    fn default() -> Self {
        Catalog {
            open: false,
            kind: Kind::Builtin,
            search: String::new(),
            selected: Vec::new(),
            scroll: 0.0,
            content: 0.0,
            offset: Vec2::ZERO,
            placed: false,
            context: None,
            pending_delete: None,
        }
    }
}

impl Catalog {
    /// 消えたブラシを、選び・右クリックの対象から外す。
    pub fn forget(&mut self, key: BrushKey) {
        if let BrushKey::User(id) = key {
            self.selected.retain(|i| *i != CatalogItem::User(id));
            if self.context == Some(CatalogItem::User(id)) {
                self.context = None;
            }
        }
    }

    pub fn toggle(&mut self, item: CatalogItem) {
        match self.selected.iter().position(|i| *i == item) {
            Some(at) => {
                self.selected.remove(at);
            }
            None => self.selected.push(item),
        }
    }

    pub fn is_selected(&self, item: CatalogItem) -> bool {
        self.selected.contains(&item)
    }
}

/// 一覧の 1 行。
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub item: CatalogItem,
    pub name: String,
    /// もう並びにある（追加すると写しを作る）。
    pub placed: bool,
}

fn matches(name: &str, needle: &str) -> bool {
    needle.is_empty() || name.to_lowercase().contains(needle)
}

/// 種類 `kind` の行（名前の検索 `search` に合う物。組み込みと同梱の Krita はその並び、利用者のブラシは名前の順）。同梱の Krita は
/// 読み込み済みのときだけ（読み込みは画面が `KritaTips::poll` で始める）。
pub fn rows(app: &AppState, kind: Kind, search: &str) -> Vec<Row> {
    let lang = app.lang;
    let needle = search.trim().to_lowercase();
    let set = &app.toolset.set;
    match kind {
        Kind::Builtin => builtin::all()
            .iter()
            .map(|b| Row {
                item: CatalogItem::Builtin(b.id),
                name: builtin::name(lang, b.id),
                placed: set.contains(BrushKey::Builtin(b.id)),
            })
            .filter(|r| matches(&r.name, &needle))
            .collect(),
        Kind::Krita => match store::krita_loaded() {
            Some(bundle) => bundle
                .brushes
                .iter()
                .enumerate()
                .filter(|(_, b)| matches(&b.name, &needle))
                .map(|(i, b)| Row {
                    item: CatalogItem::Krita(i),
                    name: b.name.clone(),
                    placed: false,
                })
                .collect(),
            None => Vec::new(),
        },
        _ => {
            let mut rows: Vec<Row> = app
                .brushes
                .lib
                .entries()
                .iter()
                .filter(|e| e.key.is_user() && kind_of(e) == kind)
                .filter(|e| matches(&e.name, &needle))
                .filter_map(|e| match e.key {
                    BrushKey::User(id) => Some(Row {
                        item: CatalogItem::User(id),
                        name: e.name.clone(),
                        placed: set.contains(e.key),
                    }),
                    BrushKey::Builtin(_) => None,
                })
                .collect();
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            rows
        }
    }
}

/// 行の見本に使う設定（同梱の Krita は読み込み済みのときだけ）。
pub fn brush_of(app: &AppState, item: CatalogItem) -> Option<crate::engine::Brush> {
    match item {
        CatalogItem::Krita(i) => store::krita_loaded()?
            .brushes
            .get(i)
            .map(|b| crate::brushes::canonical(&b.brush)),
        other => app
            .brushes
            .lib
            .entry(other.key()?)
            .map(|e| e.baseline.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_a_name_in_both_languages() {
        for kind in Kind::ALL {
            for lang in [Lang::Ja, Lang::En] {
                assert!(!kind.name(lang).is_empty());
            }
            assert!(!kind
                .name(Lang::En)
                .chars()
                .any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)));
        }
    }

    #[test]
    fn selection_toggles_and_forgets_deleted_files() {
        let mut c = Catalog::default();
        c.toggle(CatalogItem::User(3));
        c.toggle(CatalogItem::Builtin("pencil"));
        assert!(c.is_selected(CatalogItem::User(3)));
        c.toggle(CatalogItem::Builtin("pencil"));
        assert_eq!(c.selected, [CatalogItem::User(3)]);
        c.context = Some(CatalogItem::User(3));
        c.forget(BrushKey::User(3));
        assert!(c.selected.is_empty());
        assert_eq!(c.context, None);
    }
}
