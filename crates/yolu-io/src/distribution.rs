//! 配布用の写し: 開いている .ylp から、作った人が気づかないまま残る物を除いた写しを作る。
//!
//! .ylp は販売物に含まれることがある。作業用のファイルには、取り込んだ PSD の原本・どのレイヤーにも使われていない棚の素材・取り込んだ元のパス・モデルの
//! 場所・焼いたメッシュマップ・Unity のマテリアルの値・古い状態のエントリ・知らないエントリが残るので、写しを書くときに種類ごとに除く
//! （[`Removal`]。目録は [`Project::distribution_inventory`]、写しは [`Project::for_distribution`]）。作業用のファイルは変えない。
//!
//! 写しは今のファイルと同じ形式（形式 7。名前を付けて残した選択範囲を残す写しは 8。正本の版は変えない）で、除くのは次の物だけ。除くものは、Unity 版（0.2.0）の読み手がどれも無くてよい（PSD の原本・
//! 状態のエントリ・派生のエントリ）か、索引と合わせて書き直す（棚）物だけで、`resources.json` の並びにある物は並びと中身が必ず揃う（並びが
//! 空になれば `resources.json` を書かない）。ただし正本の版は変えないので、Unity 版 0.2.0 が開けるのは正本の版 21 のセットだけを含む写しで、
//! 版 22 以上のセットを含む写しは Unity 版 0.2.0 では開けない（`docs/YLP_FORMAT.md` の「読み手ごとの範囲」）。
//! Unity 版の読み手のコードと形式の決まりを読んで確かめたもので、Unity では開いていない。スタンドアロン版では、写しを読み直す試験で確かめる。
//!
//! - PSD の原本: `sets/<ID>/imported-original.psd`。
//! - 使っていない棚の物: どのセットのレイヤー（正本の ID の項目）からも、見た目の設定（`look.json`）からも指されない `resources.json` の項目と中身。
//!   使っているかは [`Project::used_resource_ids`] の 1 か所で決める。
//! - 出どころのパス: 棚の項目の `origin` のうち `file`（絶対のパス）・`unityAsset`（Assets の中のパス）・`library`（置き場の相対パス）を `none` に。
//!   使っている棚の項目は残し、出どころだけを外す。内蔵（`builtIn`）はキーと版だけでパスではないので残す。テキストレイヤーのフォントのファイル
//!   （`font_kind` 1）の `font_path`（絶対のパス。利用者が入れたフォントは、場所にユーザー名を含みうる）は、ディレクトリを外してファイル名だけにする
//!   （`font_sha256`・`font_family`・`font_postscript`・`font_weight`・`font_italic` は残す。開くときは中身の SHA-256 と名前で探し、相対の道は使わない）。
//! - モデルの参照: `view.json` の `standaloneModel` と、モデルの GUID（空にする）、モデルの中のレンダラーの目（空にする）、根の `pose.json`
//!   （モデルのポーズ。どのモデルのものか分からなくなる）、根の `livelink.json`（Live Link で開いたモデルの記録。FBX と絵のファイルの絶対の場所・
//!   Unity のプロジェクトの場所・書き出しの置き場を持つ。ポーズも入っているので、モデルの参照と一緒に除く）。
//! - メッシュマップ: `sets/<ID>/meshmap-*.bin`（モデルの形から焼いた派生物。焼き直せる）。
//! - Unity の値: `look.json` の `received`（Live Link で Unity のマテリアルから受けた値）。利用者の設定は残る。
//! - 名前を付けて残した選択範囲: `sets/<ID>/selections.json` と `selection-*.bin`。作業の補助で、絵ではない。形式 8 のエントリなので、残した写しは
//!   Unity 版 0.2.0 以降が「新しい形式」として断る。除けば、写しの形式は 7 に戻る（今の選択範囲 `selection.bin` は絵の一部として残す）。
//! - 古い状態: 根の `thumbnail.png`・`brush.json`・`model.json`（Unity 版が残したもの。スタンドアロン版は更新しない）。
//! - 知らないエントリ: 今の形式のどの読み手も知らないエントリ（開くときに「保存すると残らない」と知らせるもの）。
//!
//! 保証しないこと: 棚の `.ylsmart`・`.ylbrush` の中身（ファイルごと残すか除くかで、中は書き換えない。中の画像の出どころは触れない）、レイヤーの名前・
//! セットの名前・マテリアルの参照（作品の一部）、今の選択範囲・合成の PNG（絵そのもの）。

use crate::{
    archive::split_set,
    bigdoc::native_entries,
    check, guid,
    package::{Blob, Files},
    project::{json, read_view, required, writer_json},
    shelf::write_index,
    NativeDocument, NativeValue, Project, Resource, Result, TextureSet, WriterInfo,
};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// 配布用の写しで除く物の種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Removal {
    /// 取り込んだ PSD の原本（`imported-original.psd`）。
    PsdOriginals,
    /// どのレイヤーからも使われていない棚の物。
    UnusedShelf,
    /// 棚の物の出どころのパス。
    SourcePaths,
    /// モデルの参照（`view.json`・`pose.json`・`livelink.json`）。
    ModelReference,
    /// 焼いたメッシュマップ。
    MeshMaps,
    /// Unity のマテリアルの値（`look.json` の `received`）。
    UnityValues,
    /// 名前を付けて残した選択範囲（`selections.json` と `selection-*.bin`。形式 8）。除くと、写しの形式は 7 に戻る。
    SavedSelections,
    /// 古い状態のエントリ（サムネイル・ブラシの設定など）。
    StaleEntries,
    /// 知らないエントリ。
    UnknownEntries,
}
impl Removal {
    /// 全部の種類（ウィンドウに並べる順）。
    pub const ALL: [Removal; 9] = [
        Removal::PsdOriginals,
        Removal::UnusedShelf,
        Removal::SourcePaths,
        Removal::ModelReference,
        Removal::MeshMaps,
        Removal::UnityValues,
        Removal::SavedSelections,
        Removal::StaleEntries,
        Removal::UnknownEntries,
    ];
}

/// 1 つの種類に当たる物（名前。PSD の原本・メッシュマップ・Unity の値はセットの名前、棚の物は項目の名前、モデルはファイルの名前、
/// エントリはエントリの名前）。名前の無い種類（モデルの GUID だけなど）は空。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub removal: Removal,
    pub names: Vec<String>,
}

/// 写しで除ける物の目録（当たるものが 1 つもない種類は含まない）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    pub found: Vec<Found>,
}
impl Inventory {
    pub fn is_empty(&self) -> bool {
        self.found.is_empty()
    }
    /// 当たる種類（`Removal::ALL` の順）。
    pub fn kinds(&self) -> Vec<Removal> {
        self.found.iter().map(|f| f.removal).collect()
    }
    pub fn get(&self, removal: Removal) -> Option<&Found> {
        self.found.iter().find(|f| f.removal == removal)
    }
}

/// 取り込んだ PSD の原本のエントリ名（セットの下）。
pub const IMPORTED_ORIGINAL: &str = "imported-original.psd";
/// Unity 版が残し、スタンドアロン版は更新しない根の状態・派生のエントリ。
const STALE_ROOT: [&str; 3] = ["thumbnail.png", "brush.json", "model.json"];

fn set_entry(id: &str, leaf: &str) -> String {
    format!("sets/{id}/{leaf}")
}
fn is_mesh_map(leaf: &str) -> bool {
    leaf.starts_with("meshmap-") && leaf.ends_with(".bin")
}
/// 出どころがパス（絶対・Assets の中・置き場の相対）か。
fn has_source_path(origin: &Value) -> bool {
    matches!(
        origin.get("type").and_then(Value::as_str),
        Some("file" | "unityAsset" | "library")
    )
}
fn file_name_of(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(path)
        .to_owned()
}

/// テキストレイヤーのフォントのファイルの場所の項目名の終わり（正本の項目の道は `layers[N].text.font_path`）。
const FONT_PATH_FIELD: &str = ".text.font_path";

/// 正本の骨組みにある、ディレクトリを持つフォントの道（項目の道と、今の値）。ファイル名だけのものは除く。
fn font_paths_of(document: &NativeDocument) -> Vec<(String, String)> {
    document
        .fields()
        .iter()
        .filter(|f| f.path.starts_with("layers[") && f.path.ends_with(FONT_PATH_FIELD))
        .filter_map(|f| match &f.value {
            NativeValue::Text(p) if file_name_of(p) != *p => Some((f.path.clone(), p.clone())),
            _ => None,
        })
        .collect()
}

// ───────── 参照の集め（使っている棚の物の判定は、ここ 1 か所） ─────────

/// 棚の ID の照合用の鍵: ハイフンを除いた小文字の 16 進 32 桁（`look.json` の画像の ID は 32 桁、正本は GUID、索引は 36 桁）。
fn key_of(text: &str) -> Option<String> {
    let key: String = text
        .chars()
        .filter(|c| *c != '-')
        .flat_map(char::to_lowercase)
        .collect();
    (key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())).then_some(key)
}

struct References {
    by_key: HashMap<String, String>,
    used: BTreeSet<String>,
}
impl References {
    fn new(resources: &[Resource]) -> Self {
        Self {
            by_key: resources
                .iter()
                .filter_map(|r| key_of(&r.id).map(|k| (k, r.id.clone())))
                .collect(),
            used: BTreeSet::new(),
        }
    }
    fn note(&mut self, text: &str) {
        if let Some(id) = key_of(text).and_then(|k| self.by_key.get(&k)) {
            self.used.insert(id.clone());
        }
    }
    /// 文字列の中の、16 進とハイフンだけが 32 文字以上続く所を ID の候補として照合する（JSON の構造を見ない: 知らない形のキーの中の
    /// ID も拾い、使っている物を見落とさない）。連なり全体に加え、ID の前後に 16 進の文字（`face<ID>`・`<ID>-1`）やハイフンが隣り合う
    /// ときのために、36 文字（ハイフンつき）と 32 文字（ハイフン無し）のウィンドウも当てる。長さが 32 文字以上なら、短い連なりでも必ずウィンドウを当てる。
    fn scan_text(&mut self, bytes: &[u8]) {
        let is_id_byte = |b: &u8| b.is_ascii_hexdigit() || *b == b'-';
        let mut at = 0;
        while at < bytes.len() {
            if !is_id_byte(&bytes[at]) {
                at += 1;
                continue;
            }
            let start = at;
            while at < bytes.len() && is_id_byte(&bytes[at]) {
                at += 1;
            }
            let run = &bytes[start..at];
            if run.len() < 32 || run.len() > 1 << 16 {
                continue;
            }
            // ASCII だけの連なりなので UTF-8
            let run = std::str::from_utf8(run).expect("16 進とハイフンは ASCII");
            if run.len() <= 36 {
                self.note(run);
            }
            for width in [36, 32] {
                for window in run.as_bytes().windows(width) {
                    self.note(std::str::from_utf8(window).expect("ASCII"));
                }
            }
        }
    }
    fn scan_documents(&mut self, sets: &[TextureSet]) {
        for set in sets {
            // 正本の骨組み（ID と文字列の項目）を見る。読めない正本は、使っている物を見落とさないよう、棚の全部を使っているとみなす
            let Ok(skeleton) = set.document.skeleton() else {
                self.used.extend(self.by_key.values().cloned());
                continue;
            };
            for field in skeleton.fields() {
                match &field.value {
                    NativeValue::Guid(g) => self.note(&guid(g)),
                    NativeValue::Text(t) => self.scan_text(t.as_bytes()),
                    _ => {}
                }
            }
        }
    }
    fn scan_looks(&mut self, sets: &[TextureSet], files: &Files) {
        for set in sets {
            if let Some(blob) = files.get(&set_entry(&set.id, crate::look::ENTRY)) {
                match blob.bytes() {
                    Ok(bytes) => self.scan_text(&bytes),
                    Err(_) => self.used.extend(self.by_key.values().cloned()),
                }
            }
        }
    }
}

impl Project {
    /// 棚の項目のうち、どれかのセットのレイヤー（正本の中の ID の項目。塗りつぶしの画像・グラデーションの形・Generator・マスクなど、正本が棚を
    /// ID で指すもの全部）か、見た目の設定（`look.json` の画像の割り当て。`received` の中も）が指すものの ID。配布用の写しで「使っている」の
    /// 判定は、これと同じ集め方（[`Project::used_in`]）の 1 か所だけで決める。
    pub fn used_resource_ids(&self) -> BTreeSet<String> {
        self.used_in(&self.files)
    }

    /// `files`（このプロジェクトのエントリから、除いた後のもの）の見た目の設定を見て、使っている棚の項目の ID を集める（正本はセットのもの）。
    fn used_in(&self, files: &Files) -> BTreeSet<String> {
        let mut references = References::new(&self.resources);
        references.scan_documents(&self.sets);
        references.scan_looks(&self.sets, files);
        references.used
    }

    /// 写しで除ける物の目録。当たる物が無い種類は含まない。何も変えない。`remove` は利用者が選んでいる除く種類: 使っているかは、除いた後に残る
    /// 見た目の設定（Unity の値を除くなら、その中の割り当ては数えない）で決まるので、使っていない棚の物の名前はこれで変わる（残す種類を選べば、
    /// 写しでも残る物と一致する）。ほかの種類は、選びに依らずプロジェクトにあるもの。
    pub fn distribution_inventory(&self, remove: &[Removal]) -> Inventory {
        let mut found = Vec::new();
        let mut add = |removal: Removal, names: Vec<String>| found.push(Found { removal, names });
        let sets_with = |has: &dyn Fn(&TextureSet) -> bool| -> Vec<String> {
            self.sets
                .iter()
                .filter(|s| has(s))
                .map(|s| s.name.clone())
                .collect()
        };
        let psd = sets_with(&|s| {
            self.files
                .contains_key(&set_entry(&s.id, IMPORTED_ORIGINAL))
        });
        if !psd.is_empty() {
            add(Removal::PsdOriginals, psd);
        }
        // 除けない見た目の設定（読めないもの）は除かずに数える（使っている物を見落とさない）
        let after = self
            .stripped_files(remove)
            .unwrap_or_else(|_| self.original.files.clone());
        let used = self.used_in(&after);
        let unused: Vec<String> = self
            .resources
            .iter()
            .filter(|r| !used.contains(&r.id))
            .map(|r| r.name.clone())
            .collect();
        if !unused.is_empty() {
            add(Removal::UnusedShelf, unused);
        }
        let mut paths: Vec<String> = self
            .resources
            .iter()
            .filter(|r| r.metadata.get("origin").is_some_and(has_source_path))
            .map(|r| r.name.clone())
            .collect();
        // テキストレイヤーのフォントのファイル（場所を、ファイル名だけにする）
        for set in &self.sets {
            if let Ok(skeleton) = set.document.skeleton() {
                for (_, path) in font_paths_of(&skeleton) {
                    let name = file_name_of(&path);
                    if !paths.contains(&name) {
                        paths.push(name);
                    }
                }
            }
        }
        if !paths.is_empty() {
            add(Removal::SourcePaths, paths);
        }
        if let Some(names) = self.model_reference() {
            add(Removal::ModelReference, names);
        }
        let maps = sets_with(&|s| {
            let prefix = set_entry(&s.id, "");
            self.files
                .keys()
                .any(|n| n.strip_prefix(&prefix).is_some_and(is_mesh_map))
        });
        if !maps.is_empty() {
            add(Removal::MeshMaps, maps);
        }
        let received = sets_with(&|s| {
            self.files
                .get(&set_entry(&s.id, crate::look::ENTRY))
                .is_some_and(|b| b.bytes().is_ok_and(|b| crate::look::has_received(&b)))
        });
        if !received.is_empty() {
            add(Removal::UnityValues, received);
        }
        let remembered = sets_with(&|s| {
            let prefix = set_entry(&s.id, "");
            self.files.keys().any(|n| {
                n.strip_prefix(&prefix)
                    .is_some_and(crate::saved_selections::is_entry_leaf)
            })
        });
        if !remembered.is_empty() {
            add(Removal::SavedSelections, remembered);
        }
        let stale: Vec<String> = STALE_ROOT
            .iter()
            .filter(|n| self.files.contains_key(**n))
            .map(|n| (*n).to_owned())
            .collect();
        if !stale.is_empty() {
            add(Removal::StaleEntries, stale);
        }
        if !self.unknown.is_empty() {
            add(Removal::UnknownEntries, self.unknown.clone());
        }
        Inventory { found }
    }

    /// モデルの参照（`view.json` のモデルの場所・Unity のモデルの GUID・レンダラーの目と、Live Link で開いたモデルの記録 `livelink.json`）が
    /// あれば、その名前（モデルのファイルの名前、記録はエントリの名前 `livelink.json`。名前が無ければ空）。読めない `view.json` は、中を確かめ
    /// られないので名前 `view.json` で当たる。
    fn model_reference(&self) -> Option<Vec<String>> {
        let view = self.view_model_reference();
        let link = self.files.contains_key(crate::livelink::ENTRY);
        if view.is_none() && !link {
            return None;
        }
        let mut names = view.unwrap_or_default();
        if link {
            names.push(crate::livelink::ENTRY.into());
        }
        Some(names)
    }

    /// `view.json` にモデルの参照があれば、その名前（モデルのファイルの名前。名前が無ければ空）。
    fn view_model_reference(&self) -> Option<Vec<String>> {
        let bytes = self.files.get("view.json")?;
        let Ok(view) = bytes.bytes().and_then(|b| read_view(&b)) else {
            return Some(vec!["view.json".into()]);
        };
        let model = view.get("standaloneModel").filter(|m| !m.is_null());
        let guid_set = view
            .get("modelAssetGuid")
            .and_then(Value::as_str)
            .is_some_and(|g| !g.is_empty());
        let eyes = view
            .pointer("/visibility/hiddenRenderers")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());
        if model.is_none() && !guid_set && !eyes {
            return None;
        }
        Some(
            model
                .and_then(|m| m.get("path"))
                .and_then(Value::as_str)
                .map(file_name_of)
                .into_iter()
                .collect(),
        )
    }

    /// 取り込んだ PSD の原本（`imported-original.psd`）を、そのセットから除いたプロジェクト。文書を別の物に替えたセット（PSD を「今のセットへ」
    /// 取り込み直したなど）の保存に使う: 古い原本は新しい文書の原本ではない。無ければそのまま。形式 7 だけ（旧形式は先に upgraded）。
    pub fn without_imported_original(&self, set_id: &str) -> Result<Self> {
        check(
            self.is_current(),
            "PSD の原本を除く前にupgradedで形式7へ移行してください",
        )?;
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let name = set_entry(set_id, IMPORTED_ORIGINAL);
        if !self.original.files.contains_key(&name) {
            return Ok(self.clone());
        }
        let mut files = self.original.files.clone();
        files.remove(&name);
        self.rebuild_at(files)
    }

    /// 配布用の写し: `remove` の種類を除いた形式 7 のプロジェクト。このプロジェクトは変えない。`savedBy` は writer にする。書いたものは読み直して
    /// 検証する（索引にある物は中身が揃う）。形式 7 だけ（旧形式は先に upgraded）。
    pub fn for_distribution(&self, writer: WriterInfo, remove: &[Removal]) -> Result<Self> {
        check(
            self.is_current(),
            "配布用の写しを作る前にupgradedで形式7へ移行してください",
        )?;
        let on = |r: Removal| remove.contains(&r);
        let mut files = self.stripped_files(remove)?;
        self.write_shelf(
            &mut files,
            on(Removal::UnusedShelf),
            on(Removal::SourcePaths),
        )?;
        if on(Removal::SourcePaths) {
            self.strip_font_paths(&mut files)?;
        }
        let mut info = json(&required(&files, "ylp.json")?, 65536)?;
        info["savedBy"] = writer_json(&writer);
        files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info)?));
        self.rebuild_at(files)
    }

    /// 棚以外の種類（`remove` のうち）を除いたエントリ。
    fn stripped_files(&self, remove: &[Removal]) -> Result<Files> {
        let on = |r: Removal| remove.contains(&r);
        let mut files = self.original.files.clone();
        if on(Removal::PsdOriginals) {
            for set in &self.sets {
                files.remove(&set_entry(&set.id, IMPORTED_ORIGINAL));
            }
        }
        if on(Removal::MeshMaps) {
            files.retain(|n, _| !split_set(n).is_some_and(|(_, leaf)| is_mesh_map(leaf)));
        }
        if on(Removal::UnityValues) {
            for set in &self.sets {
                let name = set_entry(&set.id, crate::look::ENTRY);
                let Some(previous) = files.get(&name).map(Blob::bytes).transpose()? else {
                    continue;
                };
                if !crate::look::has_received(&previous) {
                    continue;
                }
                match crate::look::write_received(None, Some(&previous[..]))
                    .map_err(|e| e.in_context(format!("セット「{}」の見た目の設定", set.name)))?
                {
                    Some(bytes) => {
                        files.insert(name, Blob::from(bytes));
                    }
                    None => {
                        files.remove(&name);
                    }
                }
            }
        }
        if on(Removal::SavedSelections) {
            files.retain(|n, _| {
                !split_set(n).is_some_and(|(_, leaf)| crate::saved_selections::is_entry_leaf(leaf))
            });
        }
        if on(Removal::StaleEntries) {
            for name in STALE_ROOT {
                files.remove(name);
            }
        }
        if on(Removal::UnknownEntries) {
            for name in &self.unknown {
                files.remove(name);
            }
        }
        if on(Removal::ModelReference) {
            // モデルのポーズ（pose.json）も、モデルの参照と一緒に除く（どのモデルのポーズか分からなくなる）。Live Link で開いたモデルの記録
            // （livelink.json）は、FBX と絵のファイルの絶対の場所・Unity のプロジェクトの場所・書き出しの置き場を持つ（ポーズも持つ）ので同じく除く
            files.remove(crate::pose::ENTRY);
            files.remove(crate::livelink::ENTRY);
            if let Some(blob) = files.get("view.json").cloned() {
                match blob.bytes().and_then(|b| read_view(&b)) {
                    Ok(mut view) => {
                        clean_view(&mut view);
                        files.insert(
                            "view.json".into(),
                            Blob::from(serde_json::to_vec_pretty(&view)?),
                        );
                    }
                    // 読めない状態は中を確かめられないので、エントリごと除く（状態のエントリは無くても開ける）
                    Err(_) => {
                        files.remove("view.json");
                    }
                }
            }
        }
        Ok(files)
    }

    /// テキストレイヤーのフォントのファイルの道を、ファイル名だけにする（正本を書き直す。変わらないセットの正本は元のバイト列のまま）。
    /// 正本が大きすぎてメモリに読めないセット（512 MiB を超える）は、書き直せないので断る（場所を黙って残さない）。
    fn strip_font_paths(&self, files: &mut Files) -> Result<()> {
        for set in &self.sets {
            let changes: Vec<(String, NativeValue)> = font_paths_of(&*set.document.skeleton()?)
                .into_iter()
                .map(|(field, path)| (field, NativeValue::Text(file_name_of(&path))))
                .collect();
            if changes.is_empty() {
                continue;
            }
            let rewritten = set
                .document
                .to_native()
                .and_then(|native| native.with_values(&changes))
                .map_err(|e| {
                    e.in_context(format!(
                        "セット「{}」のテキストのフォントの場所を写しから外せません",
                        set.name
                    ))
                })?;
            let entry = set_entry(&set.id, "document.utpaint");
            files.retain(|n, _| n != &entry && !n.starts_with(&format!("{entry}.")));
            files.extend(native_entries(&rewritten, &set_entry(&set.id, "")));
        }
        Ok(())
    }

    /// 棚の索引と中身を、使っていない物（`drop_unused`）と出どころのパス（`clear_paths`）を除いて書き直す（変わらなければ元のバイト列のまま）。
    fn write_shelf(&self, files: &mut Files, drop_unused: bool, clear_paths: bool) -> Result<()> {
        let used = self.used_in(files);
        let mut kept: Vec<Resource> = Vec::with_capacity(self.resources.len());
        let mut cleared = false;
        for r in &self.resources {
            if drop_unused && !used.contains(&r.id) {
                continue;
            }
            let mut r = r.clone();
            if clear_paths && r.metadata.get("origin").is_some_and(has_source_path) {
                r.metadata["origin"] = serde_json::json!({ "type": "none" });
                cleared = true;
            }
            kept.push(r);
        }
        if kept.len() == self.resources.len() && !cleared {
            return Ok(());
        }
        // 同じ中身を指す項目が残るなら、その中身は残す
        for r in &self.resources {
            if !kept.iter().any(|k| k.entry == r.entry) {
                files.remove(&r.entry);
            }
        }
        if kept.is_empty() {
            // リソースの無いプロジェクトは resources.json を持たない
            files.remove("resources.json");
        } else {
            files.insert("resources.json".into(), Blob::from(write_index(&kept)?));
        }
        Ok(())
    }
}

/// `view.json` のモデルの参照を外す: `standaloneModel` を消し、モデルの GUID とモデルの中のレンダラーの目を空にする（Unity 版の読み手は
/// 空の GUID・空の配列を、モデル無しの状態として読む）。
fn clean_view(view: &mut Value) {
    let Some(object) = view.as_object_mut() else {
        return;
    };
    object.remove("standaloneModel");
    if object.contains_key("modelAssetGuid") {
        object.insert("modelAssetGuid".into(), Value::from(""));
    }
    if let Some(visibility) = object.get_mut("visibility").and_then(Value::as_object_mut) {
        if visibility.contains_key("hiddenRenderers") {
            visibility.insert("hiddenRenderers".into(), Value::Array(Vec::new()));
        }
    }
}
