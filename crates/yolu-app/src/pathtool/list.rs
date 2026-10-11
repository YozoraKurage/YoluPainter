//! レイヤーのパスの一覧の操作（名前・表示・写す・貼り付け・複製・並べ替え・消す・設定や位置だけを別のパスへ写す）。文書を変える操作は、
//! 一覧を描き直して 1 回の Undo（`AppState::path_write_list`）。写すだけは文書を変えず、ツールの状態の写しの場所に置く。
//!
//! 一覧は下から順に描く（後のパスが上）。画面の一覧は上が後のパス（レイヤーの一覧と同じ向き）。名前の無いパスは、画面が下からの
//! 並びの番号で名前を作る（「パス 2」）。

use yolu_core::paths::{CanvasPath, LayerPathEntry, SurfacePath, MAX_LAYER_PATHS, MAX_PATH_NAME};
use yolu_core::{LayerId, LayerPath};

use super::{random_id, ActivePath, PathAction};
use crate::lang::Lang;
use crate::notice::Source;
use crate::state::AppState;

/// 並べ替えの向き（一覧の上へ = 後に描く）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Toward {
    Up,
    Down,
}

/// 一覧の操作（パスは ID で指す）。
#[derive(Clone, Debug, PartialEq)]
pub enum ListOp {
    /// 名前を変える（空なら番号の名前に戻す）。
    Rename(u128, String),
    /// 描くか（隠したパスは画素に入らないが、点と設定は残る）。
    Visible(u128, bool),
    /// 写しの場所へ写す（文書は変えない）。
    Copy(u128),
    /// 写したパスを、選んでいるレイヤーの一覧の上に加える（新しい ID）。パスの無いレイヤーなら、新しいパスレイヤーを作る。
    Paste,
    /// すぐ上に写しを加える（新しい ID）。
    Duplicate(u128),
    Move(u128, Toward),
    Delete(u128),
    /// 写したパスの設定（ブラシ・組）だけを、このパスへ写す（点は変えない）。
    PasteSettings(u128),
    /// 写したパスの点の位置だけを、このパスへ写す（ブラシ・組は変えない）。
    PastePositions(u128),
}

impl ListOp {
    /// 文書を変える操作か（写すだけは変えない）。
    pub fn edits_document(&self) -> bool {
        !matches!(self, ListOp::Copy(_))
    }
}

/// 一覧の行の名前（名前が無ければ、下からの番号で「パス n」）。
pub fn display_name(lang: Lang, entries: &[LayerPathEntry], index: usize) -> String {
    match entries.get(index) {
        Some(e) if !e.name.is_empty() => e.name.clone(),
        _ => format!("{} {}", lang.pick("パス", "Path"), index + 1),
    }
}

/// 写しの名前（名前のあるパスだけ「〜のコピー」。番号の名前は付け直す）。上限を超える名前は切る。
fn copy_name(lang: Lang, name: &str) -> String {
    if name.is_empty() {
        return String::new();
    }
    let full = lang.pick(format!("{name} のコピー"), format!("{name} copy"));
    clip_name(&full)
}

/// 名前を上限（UTF-16 で 128 単位）に収め、制御文字を除く。
pub fn clip_name(name: &str) -> String {
    let mut out = String::new();
    let mut units = 0;
    for c in name.trim().chars().filter(|c| !c.is_control()) {
        units += c.len_utf16();
        if units > MAX_PATH_NAME {
            break;
        }
        out.push(c);
    }
    out
}

/// 設定（ブラシ・組）を `from` から写し、`to` の ID・点・基準のチャンネル・指紋は残す。側（2D・3D）が違えば None。
fn with_settings(to: &LayerPath, from: &LayerPath) -> Option<LayerPath> {
    match (to, from) {
        (LayerPath::Canvas(t), LayerPath::Canvas(f)) => Some(LayerPath::Canvas(CanvasPath {
            brush: f.brush,
            material: f.material.clone(),
            ..t.clone()
        })),
        (LayerPath::Surface(t), LayerPath::Surface(f)) => Some(LayerPath::Surface(SurfacePath {
            brush: f.brush,
            material: f.material.clone(),
            ..t.clone()
        })),
        _ => None,
    }
}

/// 点の位置を `from` から写し、`to` のほかは残す。側が違う・3D でモデルが違えば None。
fn with_positions(to: &LayerPath, from: &LayerPath) -> Option<LayerPath> {
    match (to, from) {
        (LayerPath::Canvas(t), LayerPath::Canvas(f)) => Some(LayerPath::Canvas(CanvasPath {
            points: f.points.clone(),
            ..t.clone()
        })),
        (LayerPath::Surface(t), LayerPath::Surface(f))
            if t.model_fingerprint == f.model_fingerprint =>
        {
            Some(LayerPath::Surface(SurfacePath {
                points: f.points.clone(),
                ..t.clone()
            }))
        }
        _ => None,
    }
}

impl AppState {
    /// 一覧のパスを選ぶ（None ならパスの編集を抜ける）。選んだ点は外す。
    pub(super) fn path_select_path(&mut self, id: Option<u128>) {
        let Some(layer) = self.selected_layer else {
            return;
        };
        self.path.selected = None;
        match id {
            Some(id) if self.path_entries().iter().any(|e| e.id() == id) => {
                self.path.active = Some(ActivePath { layer, path: id });
                self.path.fresh = None;
            }
            Some(_) => {}
            None => {
                self.path_exit();
            }
        }
    }

    /// 一覧の操作を当てる。
    pub(super) fn path_list(&mut self, op: ListOp) {
        let lang = self.lang;
        if let ListOp::Copy(id) = op {
            if let Some(e) = self.path_entries().iter().find(|e| e.id() == id) {
                self.path.clipboard = Some(e.clone());
                self.info(
                    Source::Path,
                    lang.pick("パスをコピーしました。", "Path copied."),
                );
            }
            return;
        }
        // 描ける状態か（描いている間・読むだけのセット・マスクを断る）
        let Some((layer, _)) = self.path_target(None) else {
            return;
        };
        let mut entries = self
            .doc
            .layer(layer)
            .map(|l| l.paths().to_vec())
            .unwrap_or_default();
        let find = |entries: &[LayerPathEntry], id: u128| entries.iter().position(|e| e.id() == id);
        let clipboard = self.path.clipboard.clone();
        let no_clipboard = |app: &mut AppState| {
            app.refuse(
                Source::Path,
                lang.pick("コピーしたパスがありません", "No path has been copied"),
            );
        };
        // (新しい一覧, 選ぶパス)
        let (next, select): (Vec<LayerPathEntry>, Option<u128>) = match op {
            ListOp::Copy(_) => unreachable!(),
            ListOp::Rename(id, name) => {
                let Some(i) = find(&entries, id) else { return };
                let name = clip_name(&name);
                if entries[i].name == name {
                    return;
                }
                // 名前は画素を変えないので、描き直さずに名前だけを替える（3D のパスでもモデルを読んでいなくてよい）
                match self
                    .doc
                    .rename_path(layer, id, &name)
                    .map_err(|e| lang.core_error(&e))
                {
                    Ok(()) => {
                        self.modified = true;
                        self.path.selected = None;
                        self.path.active = Some(ActivePath { layer, path: id });
                        self.path.fresh = None;
                    }
                    Err(m) => self.fail(Source::Path, m),
                }
                return;
            }
            ListOp::Visible(id, on) => {
                let Some(i) = find(&entries, id) else { return };
                if entries[i].visible == on {
                    return;
                }
                entries[i].visible = on;
                (entries, None)
            }
            ListOp::Paste => {
                let Some(c) = clipboard else {
                    no_clipboard(self);
                    return;
                };
                if entries.is_empty() {
                    self.path_paste_new_layer(c);
                    return;
                }
                if entries.len() >= MAX_LAYER_PATHS {
                    self.refuse(Source::Path, too_many(lang));
                    return;
                }
                let id = random_id();
                entries.push(LayerPathEntry {
                    name: copy_name(lang, &c.name),
                    visible: c.visible,
                    path: c.path.with_id(id),
                });
                (entries, Some(id))
            }
            ListOp::Duplicate(from) => {
                let Some(i) = find(&entries, from) else {
                    return;
                };
                if entries.len() >= MAX_LAYER_PATHS {
                    self.refuse(Source::Path, too_many(lang));
                    return;
                }
                let id = random_id();
                let copy = LayerPathEntry {
                    name: copy_name(lang, &entries[i].name),
                    visible: entries[i].visible,
                    path: entries[i].path.with_id(id),
                };
                entries.insert(i + 1, copy);
                (entries, Some(id))
            }
            ListOp::Move(id, toward) => {
                let Some(i) = find(&entries, id) else { return };
                let j = match toward {
                    Toward::Up if i + 1 < entries.len() => i + 1,
                    Toward::Down if i > 0 => i - 1,
                    _ => return,
                };
                entries.swap(i, j);
                (entries, Some(id))
            }
            ListOp::Delete(id) => {
                let Some(i) = find(&entries, id) else { return };
                entries.remove(i);
                (entries, None)
            }
            ListOp::PasteSettings(id) | ListOp::PastePositions(id) => {
                let Some(c) = clipboard else {
                    no_clipboard(self);
                    return;
                };
                let Some(i) = find(&entries, id) else { return };
                let positions = matches!(op, ListOp::PastePositions(_));
                let merged = if positions {
                    with_positions(&entries[i].path, &c.path)
                } else {
                    with_settings(&entries[i].path, &c.path)
                };
                let Some(path) = merged else {
                    self.refuse(Source::Path, mismatch(lang, &entries[i].path, &c.path));
                    return;
                };
                if path == entries[i].path {
                    return;
                }
                entries[i].path = path;
                (entries, Some(id))
            }
        };
        self.path_commit_list(layer, next, select);
    }

    /// パスの一覧を書き、後始末をする: 点の選びを外し、`select` のパスを編集中にする（None なら、消えた編集中のパスだけ外す）。
    /// 面から外れて描かなかった標本があれば知らせる。書いたら true（断られたら理由を知らせて何も変えない）。
    pub(crate) fn path_commit_list(
        &mut self,
        layer: LayerId,
        next: Vec<LayerPathEntry>,
        select: Option<u128>,
    ) -> bool {
        let lang = self.lang;
        let deleted_active = self
            .path
            .active
            .is_some_and(|a| a.layer == layer && !next.iter().any(|e| e.id() == a.path));
        match self.path_write_list(layer, next) {
            Ok(gaps) => {
                self.modified = true;
                self.path.selected = None;
                if let Some(id) = select {
                    self.path.active = Some(ActivePath { layer, path: id });
                    self.path.fresh = None;
                } else if deleted_active {
                    self.path.active = None;
                }
                if gaps > 0 {
                    self.warn(Source::Path, super::gaps_message(lang, gaps));
                }
                true
            }
            Err(m) => {
                self.fail(Source::Path, m);
                false
            }
        }
    }

    /// 写したパスで、選んでいるレイヤーの上に新しいパスレイヤーを作る（名前と表示は引き継がない。レイヤーの名前が名前になる）。
    fn path_paste_new_layer(&mut self, entry: LayerPathEntry) {
        self.path_commit(None, entry.path.with_id(random_id()), None);
    }

    /// 名前を変えている行を始める（一覧の行の名前を押したとき）。
    pub fn path_begin_rename(&mut self, layer: LayerId, id: u128) {
        self.path.renaming = Some(ActivePath { layer, path: id });
        self.path.rename_started = false;
    }
}

fn too_many(lang: Lang) -> String {
    lang.with_reason(
        lang.pick("パスを追加できません", "Cannot add a path"),
        lang.pick(
            "1 つのレイヤーのパスは 256 本までです",
            "a layer holds up to 256 paths",
        ),
    )
}

/// 設定・位置を写せない理由。
fn mismatch(lang: Lang, to: &LayerPath, from: &LayerPath) -> String {
    let why = if to.is_canvas() != from.is_canvas() {
        lang.pick(
            "キャンバスのパスとモデルの上のパスの間では写せません",
            "paths on the canvas and on the model do not share them",
        )
    } else {
        lang.pick(
            "別のモデルで描かれたパスです",
            "the path was drawn on another model",
        )
    };
    lang.with_reason(lang.pick("貼り付けられません", "Cannot paste"), why)
}

impl From<ListOp> for PathAction {
    fn from(op: ListOp) -> PathAction {
        PathAction::List(op)
    }
}
