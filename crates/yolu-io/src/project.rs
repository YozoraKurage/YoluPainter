use crate::{
    archive::split_set,
    bigdoc::{DocumentSource, SetDocument},
    check, check_budget, hash, is_hash,
    package::{part_number, Blob, Files, Limits, Package},
    valid_id, Archive, Error, NativeDocument, NativeValue, Result, Selection, MAX_ENTRY_BYTES,
    MAX_TOTAL_BYTES,
};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use std::{collections::HashSet, fmt, io::Cursor, sync::Arc};

/// 読める .ylp の形式の上限（8 は、名前を付けて残した選択範囲を使うファイルだけ。ほかのファイルは 7 のまま）。
pub const MAX_FORMAT: i32 = 8;
/// 名前を付けて残した選択範囲を使うファイルの形式。
pub const SAVED_SELECTIONS_FORMAT: i32 = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterInfo {
    pub app: String,
    pub version: String,
    pub unity: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatInfo {
    pub format: i32,
    pub saved_by: Option<WriterInfo>,
    pub created_by: Option<WriterInfo>,
}
/// 形式7のマテリアル参照。アセット参照がない名前は重複してよい。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialRef {
    Material {
        name: String,
        asset: Option<MaterialAsset>,
    },
    Unassigned,
    PendingSlot(u16),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialAsset {
    pub guid: String,
    pub file_id: i64,
}
impl MaterialRef {
    fn read(v: &Value) -> Result<Self> {
        check(v.is_object(), "material がオブジェクトではありません")?;
        let unassigned = match v.get("unassigned") {
            None => false,
            Some(x) => x
                .as_bool()
                .ok_or_else(|| Error::InvalidData("unassigned が真偽値ではありません".into()))?,
        };
        check(
            usize::from(unassigned)
                + usize::from(v.get("slot").is_some())
                + usize::from(v.get("name").is_some())
                == 1,
            "material の種類を一つ指定してください",
        )?;
        if unassigned {
            return Ok(Self::Unassigned);
        }
        if v.get("slot").is_some() {
            return Ok(Self::PendingSlot(number(v, "slot", 0, 65535)? as u16));
        }
        let name = text(v, "name", 0, 256)?.to_string();
        check(
            !name.chars().any(|c| c < ' ' || c == '\x7f'),
            "マテリアル名に制御文字があります",
        )?;
        check(
            v.get("guid").is_some() == v.get("fileId").is_some(),
            "guid と fileId は対で指定してください",
        )?;
        let asset = if v.get("guid").is_some() {
            let guid = text(v, "guid", 32, 32)?.to_string();
            check(
                guid.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "マテリアルのGUIDが不正です",
            )?;
            Some(MaterialAsset {
                guid,
                file_id: number(v, "fileId", i64::MIN, i64::MAX)?,
            })
        } else {
            None
        };
        Ok(Self::Material { name, asset })
    }
    fn to_json(&self) -> Value {
        match self {
            Self::Unassigned => serde_json::json!({"unassigned": true}),
            Self::PendingSlot(slot) => serde_json::json!({"slot": slot}),
            Self::Material { name, asset: None } => serde_json::json!({"name": name}),
            Self::Material {
                name,
                asset: Some(asset),
            } => serde_json::json!({"name": name, "guid": asset.guid, "fileId": asset.file_id}),
        }
    }
    fn exclusive_key(&self) -> Option<String> {
        match self {
            Self::Unassigned => Some("unassigned".into()),
            Self::PendingSlot(slot) => Some(format!("slot:{slot}")),
            Self::Material { asset: Some(a), .. } => {
                Some(format!("asset:{}:{}", a.guid, a.file_id))
            }
            Self::Material { asset: None, .. } => None,
        }
    }
}
#[derive(Clone, Debug)]
pub struct TextureSet {
    pub id: String,
    pub name: String,
    pub material: MaterialRef,
    /// 正本（画素は持たず、要るときに流して読む）。
    pub document: SetDocument,
    pub selection: Option<Selection>,
}
/// 書き手が作る・並べ直すセット（`Project::create`・`Project::with_sets`）。
#[derive(Clone, Debug)]
pub struct SetSpec {
    pub id: String,
    pub name: String,
    pub material: MaterialRef,
    /// 新しい正本（メモリの正本か core の文書）。None なら元のプロジェクトの同じ ID のセットのエントリをバイト列のまま残す（新しい
    /// セットには要る）。core の文書は全体の正本をメモリに組まずに流して書く（大きければ版 26 で分ける）。
    pub document: Option<DocumentSource>,
    /// 新しい正本の、使っているチャンネルごとの合成の PNG（`composite/<チャンネル>.png`。`composite_pngs` が作る）。標準の
    /// チャンネルだけで、同じチャンネルを 2 回は渡せない。正本を替えたセットの `composite/` の下は、中身と合わない派生を
    /// 残さないよう全部を消してから、これを書く（Unity 版のインポーターはここからチャンネルの一覧を出す）。
    pub composites: Vec<(yolu_core::Channel, Vec<u8>)>,
}
#[derive(Clone, Debug)]
pub struct Resource {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub content: String,
    pub entry: String,
    pub metadata: Value,
}
/// 元エントリと移行後のエントリを持つ。未知のエントリ・JSONキーもそのまま保つ。
/// 開けたが気をつけることの知らせ。画面は種類から短い文を作り、`Display` は診断（日本語）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Note {
    /// view.json のマテリアルのスロットを読めず 0 を使う（中身は読めなかった理由）。
    ViewSlotUnreadable(String),
    /// 古い形式の並びを、形式 7 へメモリ上で移行した。
    Migrated { format: i32 },
    /// 古い形式のマテリアル参照を、形式 7 へメモリ上で移行した。
    MaterialRefsMigrated { format: i32 },
    /// 知らないエントリを原本のまま保持する（エントリ名）。
    UnknownEntryKept(String),
    /// セットを core へ変換できない（セット名と、core へ渡せない項目）。
    SetNotConvertible { set: String, issue: String },
    /// スマートリソースをファイルごと保持する（描画用の展開は未実装。リソース名）。
    SmartResourceKept(String),
    /// ブラシ設定と画像を原本のまま保持する。
    BrushKept,
}
impl std::fmt::Display for Note {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ViewSlotUnreadable(e) => {
                write!(f, "view.jsonのスロットを読めないため0を使います: {e}")
            }
            Self::Migrated { format } => {
                write!(f, "形式{format}を形式7の並びへメモリ上で移行しました")
            }
            Self::MaterialRefsMigrated { format } => {
                write!(
                    f,
                    "形式{format}のマテリアル参照を形式7へメモリ上で移行しました"
                )
            }
            Self::UnknownEntryKept(name) => {
                write!(f, "未対応のエントリを原本のまま保持します: {name}")
            }
            Self::SetNotConvertible { set, issue } => {
                write!(f, "セット「{set}」はcoreへ変換できません: {issue}")
            }
            Self::SmartResourceKept(name) => write!(
                f,
                "スマートリソース「{name}」をファイルごと保持します（描画用の展開は未実装）"
            ),
            Self::BrushKept => f.write_str(
                "ブラシ設定と画像を原本のまま保持します。ブラシ設定の意味の検証・実行は未実装です",
            ),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub(crate) original: Package,
    pub(crate) files: Files,
    info: FormatInfo,
    pub(crate) sets: Vec<TextureSet>,
    current: String,
    pub(crate) resources: Vec<Resource>,
    notes: Vec<Note>,
    pub(crate) unknown: Vec<String>,
}
impl Project {
    /// メモリの中の .ylp を読む（上限は既定の予算から。`Limits::default`）。
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::read_within(bytes, &Limits::default())
    }
    /// `read` の、上限（設定の予算から）を渡す形。
    pub fn read_within(bytes: &[u8], limits: &Limits) -> Result<Self> {
        Self::from_package(Package::read_bytes(bytes, limits)?, &[])
    }
    /// .ylp のファイルを流して開く（全エントリを確かめ、正本は骨組みだけを読む。画素は要るときに位置から流して読む）。
    pub fn open(path: &std::path::Path, limits: &Limits) -> Result<Self> {
        Self::from_package(Package::open(path, limits)?, &[])
    }
    /// 全エントリ（`.ylp` のエントリ名 → 中身。`ylp.json` を含み、`mimetype` と manifest は含まない）から開く。復旧の世代
    /// （`GenerationStore`）から読んだエントリを、ZIP に詰め直さずに `Project` にする。検証は `read` と同じ。
    pub fn from_entries(entries: Files) -> Result<Self> {
        Self::from_package(Package::build(entries, 3)?, &[])
    }
    /// 同じ中身を書いた外側（保存で書いたファイル）から読むプロジェクトにする。正本は中身が同じなら骨組みを読み直さない
    /// （画素はそのファイルの位置から読む。保存に使った core の文書の写しは手放す）。
    pub(crate) fn rehomed(&self, package: Package) -> Result<Self> {
        let mut known = Vec::new();
        for set in &self.sets {
            let prefix = if self.info.format < 3 {
                String::new()
            } else {
                format!("sets/{}/", set.id)
            };
            let Some(header) = package.files.get(&format!("{prefix}document.utpaint")) else {
                continue;
            };
            let mut parts = Vec::new();
            while let Some(p) = package
                .files
                .get(&format!("{prefix}document.utpaint.{}", parts.len() + 1))
            {
                parts.push(p.clone());
            }
            if let Some(doc) = set.document.repointed(header, &parts) {
                known.push(doc);
            }
        }
        Self::from_package(package, &known)
    }
    /// ファイルの位置で持つエントリの置き場の名前を `path`（保存で置き換えた後の名前）へ付け替える。読むのは開いたハンドルで、置換の
    /// あとも同じファイルを指す（保存の確かめで開いた一時ファイルのハンドルがそのまま、置き換えた後のファイルになる）ので、エントリは
    /// 向け直さない。外側・移行後のエントリ・セットの正本の同じエントリは、同じエントリ（`Blob::same`）のまま（次の作り直しで、変わって
    /// いない正本の骨組みを読み直さない）。
    pub(crate) fn moved_to(self, path: &std::path::Path) -> Self {
        self.original.note_path(path);
        for blob in self.files.values() {
            blob.note_path(path);
        }
        for set in &self.sets {
            set.document.note_path(path);
        }
        self
    }
    /// エントリを替えたプロジェクトを、今の外側の版で作り直す（配布用の写しなど）。
    pub(crate) fn rebuild_at(&self, files: Files) -> Result<Self> {
        self.rebuild(files, self.original.level, &[])
    }
    /// アセットを差し替えたエントリから作り直す（形式 7 へ上げた並び）。
    pub(crate) fn rebuild_shelf(&self, files: Files) -> Result<Self> {
        self.rebuild(files, 3, &[])
    }
    /// エントリを替えたプロジェクトを作り直す（変わっていない正本は、骨組みを読み直さずにそのまま使う）。
    fn rebuild(&self, mut files: Files, level: u32, fresh: &[SetDocument]) -> Result<Self> {
        settle_format(&mut files)?;
        let known: Vec<SetDocument> = fresh
            .iter()
            .cloned()
            .chain(self.sets.iter().map(|s| s.document.clone()))
            .collect();
        let mut package = Package::build(files, level)?;
        // 外したエントリ（替えたセットの前の正本の部分など）は、同じ中身なら圧縮したバイト列を写せる元として覚える
        let removed: Vec<&Blob> = self
            .original
            .files
            .values()
            .chain(&self.original.donors)
            .filter(|b| b.in_memory().is_none() && !package.files.values().any(|n| n.same(b)))
            .collect();
        package.remember_donors(removed.into_iter());
        Self::from_package(package, &known)
    }
    /// 今の書き手が書く形式（7 か、名前を付けて残した選択範囲を使うファイルの 8）か。旧形式は先に `upgraded` で 7 にする。
    pub(crate) fn is_current(&self) -> bool {
        self.info.format >= 7
    }
    pub fn info(&self) -> &FormatInfo {
        &self.info
    }
    pub fn sets(&self) -> &[TextureSet] {
        &self.sets
    }
    pub fn current_set(&self) -> &str {
        &self.current
    }
    /// プロジェクトの画像リソースを、効果の入力（`yolu_core::EffectInputs::with_image`）にする。画素は下の行が先（文書と同じ向き）で、
    /// `ImageInput::hash` は索引の `content` と同じ値になる。画像の ID は、正本の塗りつぶしの画像が指す ID（`yolu_core::ImageId`）と同じ。
    /// PNG が壊れていれば、どのリソースかを添えて断る。
    pub fn image_inputs(&self) -> Result<Vec<(yolu_core::ImageId, yolu_core::ImageInput)>> {
        image_inputs_of(self.resources.iter(), &self.files, 0, MAX_TOTAL_BYTES)
    }
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }
    pub fn unknown_entries(&self) -> &[String] {
        &self.unknown
    }
    /// 読んだ・作ったときの全エントリ（外側）。
    pub fn original_archive(&self) -> &Package {
        &self.original
    }
    pub fn migrated_entries(&self) -> &Files {
        &self.files
    }
    /// 読んだ形式と全エントリ内容を保って再保存する。
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        self.original.to_bytes()
    }
    /// 明示的に形式7へ移行する。正本と未知のエントリの内容は変えない。
    pub fn upgraded(&self, writer: WriterInfo) -> Result<Self> {
        let mut files = self.files.clone();
        let mut info = if let Some(b) = self.original.files.get("ylp.json") {
            json(&b.bytes()?, 65536)?
        } else {
            serde_json::json!({})
        };
        info["format"] = Value::from(7);
        let w = writer_json(&writer);
        if info.get("createdBy").is_none() {
            if let Some(w) = self
                .info
                .created_by
                .as_ref()
                .or(self.info.saved_by.as_ref())
            {
                info["createdBy"] = writer_json(w);
            } else {
                info["createdBy"] = w.clone();
            }
        }
        info["savedBy"] = w;
        files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info)?));
        self.rebuild(files, 3, &[])
    }
    /// 形式7の新しいプロジェクトを作る（セットは1〜64、どれも正本が要る）。書いたものは読み直して検証する。
    pub fn create(writer: WriterInfo, sets: &[SetSpec], current: &str) -> Result<Self> {
        let w = writer_json(&writer);
        let info = serde_json::json!({"format": 7, "savedBy": w, "createdBy": w});
        let mut files = Files::new();
        files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info)?));
        let mut list = Vec::with_capacity(sets.len());
        let mut fresh = Vec::new();
        for spec in sets {
            check(
                spec.document.is_some(),
                format!("新しいセット「{}」に正本がありません", spec.name),
            )?;
            list.push(set_json(None, spec));
            fresh.extend(put_set_entries(&mut files, spec)?);
        }
        let project = serde_json::json!({"sets": list, "current": current});
        files.insert(
            "project.json".into(),
            Blob::from(serde_json::to_vec(&project)?),
        );
        Self::from_package(Package::build(files, 3)?, &fresh)
    }
    /// 形式7のセットの並び・名前・マテリアル参照・現在のセットを置き換え、正本と合成を差し替える。旧形式は先にupgradedで
    /// 明示的に移行する。元のセットは全部が並びに要る（セットを消すのは [`Project::with_sets_dropping`] で、黙って消さない）。
    /// 並びに無かったIDは新しいセットで、正本が要る。各セットの知らないJSONキー、選択範囲・メッシュマップ・PSD原本、根のほかの
    /// エントリは残す。`savedBy` は writer にする。
    pub fn with_sets(&self, writer: WriterInfo, sets: &[SetSpec], current: &str) -> Result<Self> {
        self.with_sets_dropping(writer, sets, current, &[])
    }
    /// [`Project::with_sets`] に加えて、`dropped`（元のセットのID）のセットを消す: project.json の並びから外し、`sets/<ID>/` の下の
    /// エントリ（正本・選択範囲・合成・メッシュマップ・PSD原本）を全部消す。元のセットは、並び（`sets`）か `dropped` のどちらか
    /// 一方に要る（どちらにも無いセットは黙って消さず、両方にあるのも断る）。`dropped` にはファイルに無いIDを入れられない。
    /// 呼び手が利用者に確かめた消去だけをここへ渡す（消したセットの作業は、直前の版の退避以外には残らない）。
    pub fn with_sets_dropping(
        &self,
        writer: WriterInfo,
        sets: &[SetSpec],
        current: &str,
        dropped: &[&str],
    ) -> Result<Self> {
        check(
            self.is_current(),
            "セットを並べ直す前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        let mut project = json(&required(&files, "project.json")?, 65536)?;
        let old: Vec<Value> = project["sets"].as_array().cloned().unwrap_or_default();
        for id in dropped {
            check(
                self.sets.iter().any(|s| s.id == *id),
                format!("消すセットがファイルにありません: {id}"),
            )?;
            check(
                !sets.iter().any(|s| s.id == *id),
                format!("消すセットが並びにもあります: {id}"),
            )?;
        }
        for set in &self.sets {
            check(
                sets.iter().any(|s| s.id == set.id) || dropped.contains(&set.id.as_str()),
                format!(
                    "セット「{}」が並びにありません（消すなら消すセットに入れてください）",
                    set.name
                ),
            )?;
        }
        for id in dropped {
            let prefix = format!("sets/{id}/");
            files.retain(|n, _| !n.starts_with(&prefix));
        }
        let mut list = Vec::with_capacity(sets.len());
        let mut fresh = Vec::new();
        for spec in sets {
            let previous = old
                .iter()
                .find(|v| v["id"].as_str() == Some(spec.id.as_str()));
            check(
                previous.is_some() || spec.document.is_some(),
                format!("新しいセット「{}」に正本がありません", spec.name),
            )?;
            list.push(set_json(previous, spec));
            fresh.extend(put_set_entries(&mut files, spec)?);
        }
        project["sets"] = Value::Array(list);
        project["current"] = Value::from(current);
        files.insert(
            "project.json".into(),
            Blob::from(serde_json::to_vec(&project)?),
        );
        let mut info = json(&required(&files, "ylp.json")?, 65536)?;
        info["savedBy"] = writer_json(&writer);
        files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info)?));
        self.rebuild(files, self.original.level, &fresh)
    }
    /// 既存セットの正本だけを置き換える。選択範囲・リソースを含めて再検証する。
    pub fn with_document(&self, set_id: &str, doc: &NativeDocument) -> Result<Self> {
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let prefix = if self.info.format < 3 {
            String::new()
        } else {
            format!("sets/{set_id}/")
        };
        let mut files = self.original.files.clone();
        remove_document_entries(&mut files, &prefix);
        let (fresh, entries) =
            SetDocument::from_source(&DocumentSource::Native(doc.clone()), &prefix)?;
        files.extend(entries);
        self.rebuild(files, self.original.level, &[fresh])
    }
    /// 既存セットの選択範囲（`selection.bin`）だけを置き換える。None は選択なし（エントリを消す）。選択範囲と正本の大きさが
    /// 違えば再検証で断る。ほかのエントリには触らない。
    pub fn with_selection(&self, set_id: &str, selection: Option<&Selection>) -> Result<Self> {
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let name = if self.info.format < 3 {
            "selection.bin".into()
        } else {
            format!("sets/{set_id}/selection.bin")
        };
        let mut files = self.original.files.clone();
        match selection {
            Some(s) => {
                files.insert(name, Blob::from(s.to_bytes()));
            }
            None => {
                files.remove(&name);
            }
        }
        self.rebuild(files, self.original.level, &[])
    }
    /// セットの、名前を付けて残した選択範囲（`sets/<ID>/selections.json` と `selection-<SHA-256>.bin`。形式 8）。読めない項目は飛ばして
    /// 理由を `skipped` に返し（ファイルにはバイト列のまま残る）、読める項目だけを `items` に返す。何も無ければ空。セットが無ければ断る。
    pub fn saved_selections(
        &self,
        set_id: &str,
    ) -> Result<crate::saved_selections::SavedSelections> {
        let set = self
            .sets
            .iter()
            .find(|s| s.id == set_id)
            .ok_or_else(|| Error::InvalidData("セットがありません".into()))?;
        let size = (
            set.document.width(),
            set.document.height(),
            set.document.tile_size(),
        );
        let prefix = format!("sets/{set_id}/");
        Ok(crate::saved_selections::read(
            &|leaf| self.files.get(&format!("{prefix}{leaf}")).map(Blob::bytes),
            size,
        ))
    }
    /// 既存セットの、名前を付けて残した選択範囲だけを置き換える（前の索引と中身は全部消してから書く）。空の並びは何も書かない
    /// （エントリを消す）。ファイル全体で 1 つも使わなくなれば、形式は 7 に戻る（使うファイルだけが 8）。数・名前・重なり・文書との
    /// 大きさが決まりに合わなければ断る。形式 7 以上だけ（旧形式は先に upgraded）。ほかのエントリには触らない。
    pub fn with_saved_selections(
        &self,
        set_id: &str,
        items: &[crate::saved_selections::SavedSelection],
    ) -> Result<Self> {
        check(
            self.is_current(),
            "残した選択範囲を書く前にupgradedで形式7へ移行してください",
        )?;
        let set = self
            .sets
            .iter()
            .find(|s| s.id == set_id)
            .ok_or_else(|| Error::InvalidData("セットがありません".into()))?;
        let size = (
            set.document.width(),
            set.document.height(),
            set.document.tile_size(),
        );
        crate::saved_selections::validate(items, size)?;
        let prefix = format!("sets/{set_id}/");
        let mut files = self.original.files.clone();
        files.retain(|n, _| {
            n.strip_prefix(&prefix).is_none_or(|leaf| {
                leaf.contains('/') || !crate::saved_selections::is_entry_leaf(leaf)
            })
        });
        for (leaf, blob) in crate::saved_selections::entries(items)? {
            files.insert(format!("{prefix}{leaf}"), blob);
        }
        self.rebuild(files, self.original.level, &[])
    }
    /// モデルの今のポーズ（根の `pose.json`。状態のエントリ）。無ければ None。読めない（壊れた・新しい版・範囲外）ものはエラーを返し、
    /// エントリはバイト列のまま残る（呼び手はポーズなしで開いて理由を知らせる）。
    pub fn pose(&self) -> Result<Option<crate::pose::StoredPose>> {
        self.files
            .get(crate::pose::ENTRY)
            .map(|b| crate::pose::read(&b.bytes()?))
            .transpose()
    }
    /// 根の `pose.json` だけを置き換える（None はエントリを消す）。形式 7 以上だけ。形式も正本の版も変えない。
    pub fn with_pose(&self, pose: Option<&crate::pose::StoredPose>) -> Result<Self> {
        check(
            self.is_current(),
            "ポーズを書く前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        match pose {
            Some(p) => {
                files.insert(
                    crate::pose::ENTRY.into(),
                    Blob::from(crate::pose::write(p)?),
                );
            }
            None => {
                files.remove(crate::pose::ENTRY);
            }
        }
        self.rebuild(files, self.original.level, &[])
    }
    /// Live Link の相手の文書（根の `livelink.json`。状態のエントリ）のバイト列。無ければ None。形の確かめ（大きさ・オブジェクト・版）に
    /// 合わないものはエラーを返し、エントリはバイト列のまま残る。
    pub fn livelink(&self) -> Result<Option<Vec<u8>>> {
        let Some(blob) = self.files.get(crate::livelink::ENTRY) else {
            return Ok(None);
        };
        let bytes = blob.bytes()?.to_vec();
        crate::livelink::validate(&bytes)?;
        Ok(Some(bytes))
    }
    /// 根の `livelink.json` だけを置き換える（None はエントリを消す）。形式 7 以上だけ。形式も正本の版も変えない。
    pub fn with_livelink(&self, bytes: Option<&[u8]>) -> Result<Self> {
        check(
            self.is_current(),
            "Live Link の記録を書く前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        match bytes {
            Some(b) => {
                crate::livelink::validate(b)?;
                files.insert(crate::livelink::ENTRY.into(), Blob::from(b.to_vec()));
            }
            None => {
                files.remove(crate::livelink::ENTRY);
            }
        }
        self.rebuild(files, self.original.level, &[])
    }
    /// セットの見た目の設定（`sets/<ID>/look.json`）。無ければ None。読めない（壊れた・新しい形式の）ものはエラーを返し、元のエントリは
    /// バイト列のまま残る（呼び手は標準の見た目で開いて知らせる）。
    pub fn look(&self, set_id: &str) -> Result<Option<yolu_core::look::MaterialLook>> {
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        self.files
            .get(&format!("sets/{set_id}/{}", crate::look::ENTRY))
            .map(|b| crate::look::read(&b.bytes()?))
            .transpose()
    }
    /// セットの受けた見た目（`look.json` の `received`。Live Link で Unity のマテリアルから受けた値）。無ければ None。読めないものは
    /// エラーを返し、元のエントリはバイト列のまま残る。
    pub fn received_look(&self, set_id: &str) -> Result<Option<yolu_core::look::ReceivedLook>> {
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        match self
            .files
            .get(&format!("sets/{set_id}/{}", crate::look::ENTRY))
        {
            Some(b) => crate::look::read_received(&b.bytes()?),
            None => Ok(None),
        }
    }
    /// セットの受けた見た目だけを置き換える（None は外す）。利用者の設定と知らないキーは前のエントリのまま。利用者の設定が既定で
    /// 受けた見た目も知らないキーも無くなれば、エントリを消す。形式 7 だけ。
    pub fn with_received_look(
        &self,
        set_id: &str,
        received: Option<&yolu_core::look::ReceivedLook>,
    ) -> Result<Self> {
        check(
            self.is_current(),
            "見た目の設定を書く前にupgradedで形式7へ移行してください",
        )?;
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let name = format!("sets/{set_id}/{}", crate::look::ENTRY);
        let mut files = self.original.files.clone();
        let previous = files.get(&name).map(Blob::bytes).transpose()?;
        match crate::look::write_received(received, previous.as_deref())? {
            Some(bytes) => {
                files.insert(name, Blob::from(bytes));
            }
            None => {
                files.remove(&name);
            }
        }
        self.rebuild(files, self.original.level, &[])
    }
    /// セットの見た目の設定だけを置き換える（None は既定に戻す。受けた見た目（`received`）が無ければエントリを消す）。形式 7 だけ
    /// （旧形式は先に upgraded）。前のエントリの知らないキーと受けた見た目は残す。正本・ほかのエントリには触らない。
    pub fn with_look(
        &self,
        set_id: &str,
        look: Option<&yolu_core::look::MaterialLook>,
    ) -> Result<Self> {
        check(
            self.is_current(),
            "見た目の設定を書く前にupgradedで形式7へ移行してください",
        )?;
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let name = format!("sets/{set_id}/{}", crate::look::ENTRY);
        let mut files = self.original.files.clone();
        let previous = files.get(&name).map(Blob::bytes).transpose()?;
        match look {
            Some(look) => {
                let bytes = crate::look::write(look, previous.as_deref())?;
                files.insert(name, Blob::from(bytes));
            }
            None => match previous.filter(|b| crate::look::has_received(b)) {
                Some(previous) => {
                    let bytes = crate::look::write(
                        &yolu_core::look::MaterialLook::default(),
                        Some(&previous[..]),
                    )?;
                    files.insert(name, Blob::from(bytes));
                }
                None => {
                    files.remove(&name);
                }
            },
        }
        self.rebuild(files, self.original.level, &[])
    }
    /// セットの派生メッシュマップを読む。壊れた派生物はエラーを返し、元のエントリは保持する。
    pub fn mesh_map(
        &self,
        set_id: &str,
        kind: yolu_core::mesh_maps::MeshMapKind,
        max_bytes: usize,
    ) -> Result<Option<yolu_core::mesh_maps::BakedMeshMap>> {
        let name = self.mesh_map_entry(set_id, kind)?;
        self.original
            .files
            .get(&name)
            .map(|blob| {
                check_budget(
                    blob.len() <= max_bytes as u64,
                    "メッシュマップの予算超過です",
                )?;
                let map = crate::mesh_map::read_with_limit(&blob.bytes()?, max_bytes)?;
                check(
                    map.kind() == kind,
                    "メッシュマップのエントリ名と種類が一致しません",
                )?;
                Ok(map)
            })
            .transpose()
    }
    /// 指定した種類の派生物だけを置き換えたプロジェクトを返す。文書と未知のエントリは保つ。
    /// 書くのは形式7だけ（メッシュマップの `.bin` の版3は形式7の一部）。旧形式は先にupgradedで移行する。
    pub fn with_mesh_map(
        &self,
        set_id: &str,
        map: &yolu_core::mesh_maps::BakedMeshMap,
    ) -> Result<Self> {
        check(
            self.is_current(),
            "メッシュマップを書く前にupgradedで形式7へ移行してください",
        )?;
        let name = self.mesh_map_entry(set_id, map.kind())?;
        let mut files = self.original.files.clone();
        files.insert(name, Blob::from(crate::mesh_map::write(map)?));
        self.rebuild(files, self.original.level, &[])
    }
    fn mesh_map_entry(
        &self,
        set_id: &str,
        kind: yolu_core::mesh_maps::MeshMapKind,
    ) -> Result<String> {
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let entry = crate::mesh_map::entry_name(kind);
        Ok(if self.info.format < 3 {
            entry
        } else {
            format!("sets/{set_id}/{entry}")
        })
    }
    /// 形式7のセットのマテリアル参照を置き換える。旧形式は先にupgradedで明示的に移行する。
    pub fn with_material(&self, set_id: &str, material: MaterialRef) -> Result<Self> {
        check(
            self.is_current(),
            "マテリアル参照を変える前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        let mut project = json(&required(&files, "project.json")?, 65536)?;
        let set = project["sets"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|set| set["id"].as_str() == Some(set_id))
            .ok_or_else(|| Error::InvalidData("セットがありません".into()))?;
        // 参照の既知の鍵だけを置換し、将来の拡張項目は保持する。
        let reference = set["material"].as_object_mut().unwrap();
        for key in ["name", "guid", "fileId", "unassigned", "slot"] {
            reference.remove(key);
        }
        let Value::Object(replacement) = material.to_json() else {
            unreachable!()
        };
        reference.extend(replacement);
        files.insert(
            "project.json".into(),
            Blob::from(serde_json::to_vec(&project)?),
        );
        self.rebuild(files, self.original.level, &[])
    }
    /// スタンドアロン版が view.json（状態のエントリ）に残した、開いたモデルのファイルの参照（`standaloneModel.path`。呼び手が決めた文字列
    /// のまま。.ylp からの相対か絶対のパス）。無ければ None。読めない view.json・形の違う参照は断る（正本ではないので、呼び手は
    /// モデルなしで開いて知らせればよい）。Unity 版が書いたモデルの GUID・選んだチャンネル・表示の状態には触れない。
    pub fn view_model(&self) -> Result<Option<String>> {
        let Some(bytes) = self.files.get("view.json") else {
            return Ok(None);
        };
        let view = read_view(&bytes.bytes()?)?;
        match view.get("standaloneModel") {
            None | Some(Value::Null) => Ok(None),
            Some(model) => {
                check(
                    model.is_object(),
                    "standaloneModel がオブジェクトではありません",
                )?;
                let path = text(model, "path", 1, MODEL_PATH_MAX)?;
                check(
                    !path.chars().any(|c| c < ' ' || c == '\x7f'),
                    "モデルのパスに制御文字があります",
                )?;
                Ok(Some(path.to_owned()))
            }
        }
    }
    /// view.json にモデルのファイルの参照を書く（None なら参照を外す）。形式7だけ。view.json が無い・読めなければ Unity 版の読み手が受け入れる
    /// 最小の形（モデルの GUID は空・選んだチャンネルは Color）で作り、読めればその知らないキーも含めて残し、`standaloneModel` だけを置き換える。
    /// 状態のエントリの任意の項目を足すだけなので、形式も正本の版も変えない（Unity 版は知らない項目を読み飛ばし、次の保存で
    /// 参照を落とす。そのとき失うのはモデルの場所だけで、作業の中身は失わない）。
    pub fn with_view_model(&self, path: Option<&str>) -> Result<Self> {
        check(
            self.is_current(),
            "モデルの参照を書く前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        let minimal = || serde_json::json!({"modelAssetGuid": "", "selectedChannel": 0});
        let mut view = match (
            files
                .get("view.json")
                .map(|b| b.bytes().and_then(|b| read_view(&b))),
            path,
        ) {
            (Some(Ok(view)), _) => view,
            // 読めない view.json は状態（正本ではない）なので、参照を書くときは最小の形に作り直す（Unity 版も、読めない状態は
            // 既定に戻して開く）。参照を外すだけなら、読めないものには触らない
            (Some(Err(_)) | None, Some(_)) => minimal(),
            (Some(Err(_)), None) | (None, None) => return Ok(self.clone()),
        };
        match path {
            Some(path) => {
                check(
                    (1..=MODEL_PATH_MAX).contains(&path.encode_utf16().count())
                        && !path.chars().any(|c| c < ' ' || c == '\x7f'),
                    "モデルのパスの長さ・文字が範囲外です",
                )?;
                view["standaloneModel"] = serde_json::json!({ "path": path });
            }
            None => {
                view.as_object_mut()
                    .expect("read_view が確かめた")
                    .remove("standaloneModel");
            }
        }
        files.insert(
            "view.json".into(),
            Blob::from(serde_json::to_vec_pretty(&view)?),
        );
        self.rebuild(files, self.original.level, &[])
    }
    /// 全エントリを確かめた外側から、プロジェクトを作る。`known` の正本（同じエントリを置いたもの）は骨組みを読み直さない。
    pub(crate) fn from_package(original: Package, known: &[SetDocument]) -> Result<Self> {
        let info = if let Some(b) = original.files.get("ylp.json") {
            let root = json(&b.bytes()?, 65536)?;
            let format = number(&root, "format", 2, i32::MAX as i64)? as i32;
            let saved = writer(&root["savedBy"])?;
            let created = if root.get("createdBy").is_some_and(|v| !v.is_null()) {
                Some(writer(&root["createdBy"])?)
            } else {
                None
            };
            if format > MAX_FORMAT {
                return Err(Error::UnsupportedFormat {
                    format,
                    app: saved.app,
                    version: saved.version,
                });
            }
            FormatInfo {
                format,
                saved_by: Some(saved),
                created_by: created,
            }
        } else {
            FormatInfo {
                format: 1,
                saved_by: None,
                created_by: None,
            }
        };
        let mut files = original.files.clone();
        files.remove("ylp.json");
        let mut notes = Vec::new();
        if info.format < 3 {
            check(
                !files.contains_key("project.json")
                    && !files.keys().any(|k| k.starts_with("sets/")),
                "旧形式に移行先のセットが既にあります",
            )?;
            check(
                files.contains_key("document.utpaint"),
                "旧形式の正本がありません",
            )?;
            let doc = set_document(&files, "", known, &original.skeletons)?;
            let mut slot = 0;
            if let Some(b) = files.get("view.json") {
                match b.bytes().and_then(|b| json(&b, 65536)).and_then(|j| {
                    if j.get("materialSlot").is_none_or(Value::is_null) {
                        Ok(0)
                    } else {
                        number(&j, "materialSlot", 0, 65535)
                    }
                }) {
                    Ok(v) => slot = v,
                    Err(e) => notes.push(Note::ViewSlotUnreadable(e.to_string())),
                }
            }
            let id = doc.id();
            let names: Vec<_> = files
                .keys()
                .filter(|n| moves_into_set(n))
                .cloned()
                .collect();
            for n in names {
                let b = files.remove(&n).unwrap();
                files.insert(format!("sets/{id}/{n}"), b);
            }
            let project=format!("{{\n  \"sets\": [\n    {{ \"id\": \"{id}\", \"name\": \"Texture Set 1\", \"materialSlot\": {slot} }}\n  ],\n  \"current\": \"{id}\"\n}}\n");
            files.insert("project.json".into(), Blob::from(project.into_bytes()));
            notes.push(Note::Migrated {
                format: info.format,
            });
        }
        let mut root = json(&required(&files, "project.json")?, 65536)?;
        if info.format < 7 {
            for set in root
                .get_mut("sets")
                .and_then(Value::as_array_mut)
                .ok_or_else(|| Error::InvalidData("sets が配列ではありません".into()))?
            {
                if set.get("material").is_none() {
                    let slot = number(set, "materialSlot", 0, 65535)?;
                    set["material"] = serde_json::json!({"slot": slot});
                }
                set.as_object_mut()
                    .ok_or_else(|| Error::InvalidData("セットがオブジェクトではありません".into()))?
                    .remove("materialSlot");
            }
            files.insert(
                "project.json".into(),
                Blob::from(serde_json::to_vec(&root)?),
            );
            notes.push(Note::MaterialRefsMigrated {
                format: info.format,
            });
        }
        let list = array(&root, "sets", 1, MAX_PROJECT_SETS)?;
        let current = id_text(&root, "current")?.to_string();
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        let mut slots = HashSet::new();
        let mut sets = Vec::new();
        for s in list {
            let id = id_text(s, "id")?.to_string();
            let name = label(s, "name")?.to_string();
            let material = MaterialRef::read(&s["material"])?;
            let unique_material = material.exclusive_key().is_none_or(|k| slots.insert(k));
            check(
                ids.insert(id.clone()) && names.insert(name.to_uppercase()) && unique_material,
                "セットのID・名前・排他的なマテリアル参照が重複しています",
            )?;
            let prefix = format!("sets/{id}/");
            let document = set_document(&files, &prefix, known, &original.skeletons)?;
            let size = (document.width(), document.height(), document.tile_size());
            let selection = files
                .get(&format!("{prefix}selection.bin"))
                .map(|b| Selection::read_sized(&b.bytes()?, size))
                .transpose()?;
            sets.push(TextureSet {
                id,
                name,
                material,
                document,
                selection,
            });
        }
        check(ids.contains(&current), "現在のセットが一覧にありません")?;
        let mut budget = 0usize;
        let resources = load_resources(&files, &mut budget, 0, &mut notes)?;
        let resource_entries: HashSet<_> = resources.iter().map(|r| r.entry.as_str()).collect();
        let mut unknown = Vec::new();
        for n in files.keys() {
            let known = if let Some((id, _)) = split_set(n) {
                ids.contains(id) && entry_form(n).is_some()
            } else if n.starts_with("resources/") {
                resource_entries.contains(n.as_str())
            } else {
                entry_form(n).is_some()
            };
            if !known {
                unknown.push(n.clone());
            }
        }
        for n in &unknown {
            notes.push(Note::UnknownEntryKept(n.clone()));
        }
        for set in &sets {
            for issue in set.document.core_issues() {
                notes.push(Note::SetNotConvertible {
                    set: set.name.clone(),
                    issue,
                });
            }
        }
        Ok(Self {
            original,
            files,
            info,
            sets,
            current,
            resources,
            notes,
            unknown,
        })
    }
}
/// view.json に残すモデルのパスの長さの上限（UTF-16 の数。ファイルのパスの実用の上限に合わせる）。
pub const MODEL_PATH_MAX: usize = 1024;
/// 1 つの .ylp に入れられるテクスチャセットの数の上限（読み手が断る数）。
pub const MAX_PROJECT_SETS: usize = 64;
/// view.json の読み込み（Unity 版の予算 256 KiB と同じ。オブジェクトでなければ断る）。ここは状態のエントリなので、正本の
/// JSON の厳しい検査（`json`。文字列は 1024 文字まで）は使わない: Unity 版の表示の状態は長い鍵を持てる。
pub(crate) fn read_view(bytes: &[u8]) -> Result<Value> {
    check_budget(bytes.len() <= 262_144, "view.json のバイト予算超過です")?;
    let view: Value = serde_json::from_slice(bytes)?;
    check(view.is_object(), "view.json がオブジェクトではありません")?;
    Ok(view)
}
/// project.json のセット 1 つ。前のオブジェクトがあれば、知らないキーを残して名前とマテリアル参照の既知の鍵だけを置き換える。
fn set_json(previous: Option<&Value>, spec: &SetSpec) -> Value {
    let mut set = previous
        .cloned()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    set["id"] = Value::from(spec.id.as_str());
    set["name"] = Value::from(spec.name.as_str());
    if !set["material"].is_object() {
        set["material"] = serde_json::json!({});
    }
    let reference = set["material"].as_object_mut().unwrap();
    for key in ["name", "guid", "fileId", "unassigned", "slot"] {
        reference.remove(key);
    }
    let Value::Object(replacement) = spec.material.to_json() else {
        unreachable!()
    };
    reference.extend(replacement);
    set
}
/// セットの正本と合成を置く（正本が無ければ何もしない）。
/// セットの正本と合成を置く（正本が無ければ何もしない）。前の正本のヘッダーと部分は全部消してから置く（古い部分を残さない）。
/// 置いた正本を返す（骨組みを読み直さないため）。
fn put_set_entries(files: &mut Files, spec: &SetSpec) -> Result<Option<SetDocument>> {
    let Some(doc) = &spec.document else {
        return Ok(None);
    };
    let mut composites = Vec::with_capacity(spec.composites.len());
    for (channel, png) in &spec.composites {
        let name = channel.standard_name().ok_or_else(|| {
            Error::InvalidData(format!(
                "セット「{}」の合成のチャンネルが標準ではありません: {channel:?}",
                spec.name
            ))
        })?;
        check(
            !composites.iter().any(|(n, _)| *n == name),
            format!("セット「{}」の合成が重複しています: {name}", spec.name),
        )?;
        composites.push((name, png));
    }
    let prefix = format!("sets/{}/", spec.id);
    let (document, entries) = SetDocument::from_source(doc, &prefix)?;
    files.retain(|n, _| !n.starts_with(&format!("{prefix}composite/")));
    remove_document_entries(files, &prefix);
    files.extend(entries);
    for (name, png) in composites {
        files.insert(
            format!("{prefix}composite/{name}.png"),
            Blob::from(png.as_slice()),
        );
    }
    Ok(Some(document))
}
/// `prefix`（`sets/<ID>/` か根なら空）の下の正本のヘッダーと部分を消す。
pub(crate) fn remove_document_entries(files: &mut Files, prefix: &str) {
    files.retain(|n, _| {
        n.strip_prefix(prefix)
            .is_none_or(|leaf| leaf.contains('/') || !is_document_leaf(leaf))
    });
}
/// 正本のヘッダーか部分の名前（セットの下の葉）。
fn is_document_leaf(leaf: &str) -> bool {
    leaf == "document.utpaint" || part_number(leaf).is_some()
}
/// `prefix` の下の正本（ヘッダーと番号の続く部分）。`known` に同じエントリを置いた正本があれば、それを使う（骨組みを読み直さない）。
fn set_document(
    files: &Files,
    prefix: &str,
    known: &[SetDocument],
    read: &std::collections::BTreeMap<String, std::result::Result<Arc<NativeDocument>, String>>,
) -> Result<SetDocument> {
    let name = format!("{prefix}document.utpaint");
    let header = files
        .get(&name)
        .ok_or_else(|| Error::InvalidData(format!("エントリがありません: {name}")))?;
    let mut parts = Vec::new();
    while let Some(p) = files.get(&format!("{prefix}document.utpaint.{}", parts.len() + 1)) {
        parts.push(p.clone());
    }
    let numbered = files
        .keys()
        .filter(|n| {
            n.strip_prefix(prefix)
                .is_some_and(|leaf| !leaf.contains('/') && part_number(leaf).is_some())
        })
        .count();
    check(
        numbered == parts.len(),
        format!("正本の部分の番号が 1 から続いていません: {name}"),
    )?;
    let placed: Vec<&Blob> = std::iter::once(header).chain(&parts).collect();
    if let Some(doc) = known.iter().find(|d| d.placed_as(&placed)) {
        return Ok(doc.clone());
    }
    if parts.is_empty() {
        if let Some(skeleton) = read.get(&name) {
            let skeleton = skeleton.clone().map_err(Error::InvalidData)?;
            return Ok(SetDocument::stored_with(
                crate::bigdoc::StoredDoc {
                    header: header.clone(),
                    parts,
                },
                skeleton,
            ));
        }
    }
    SetDocument::stored(header.clone(), parts)
}
/// 根のエントリ（`resources/`・`sets/<ID>/` の下のほか）の名前。形式の仕様（`docs/YLP_FORMAT.md` の「エントリの一覧」の表）と同じ一覧で、
/// `tests/ylp/format_doc.rs` が仕様に載っているかを確かめる。エントリを足すときは、ここと仕様を同じコミットで直す。
pub const ROOT_ENTRIES: [&str; 9] = [
    "ylp.json",
    "project.json",
    "resources.json",
    "view.json",
    crate::pose::ENTRY,
    "brush.json",
    "thumbnail.png",
    "model.json",
    crate::livelink::ENTRY,
];
/// アセットの中身のエントリの形（`resources.json` の並びにあるもの。`<content>` は中身のハッシュ）。
pub const RESOURCE_ENTRIES: [&str; 3] = [
    "resources/<content>.png",
    "resources/<content>.ylsmart",
    "resources/<content>.ylbrush",
];
/// セットの下（`sets/<ID>/`）のエントリの形（`<…>` は名前ごとに変わる所）。
pub const SET_ENTRIES: [&str; 9] = [
    "document.utpaint",
    "document.utpaint.<n>",
    "selection.bin",
    "selections.json",
    "selection-<印>.bin",
    "look.json",
    "composite/<チャンネル>.png",
    "meshmap-<種類>.bin",
    "imported-original.psd",
];
/// エントリの名前が、どの形のエントリか（[`ROOT_ENTRIES`]・[`RESOURCE_ENTRIES`]・[`SET_ENTRIES`] の 1 つ。知らない名前は None）。
/// 名前の形だけを見て、並びにあるか（セットの ID・アセットの中身）は見ない。
pub fn entry_form(name: &str) -> Option<&'static str> {
    if let Some((_, leaf)) = split_set(name) {
        let form = if leaf == "document.utpaint" {
            0
        } else if part_number(leaf).is_some() {
            1
        } else if leaf == "selection.bin" {
            2
        } else if leaf == crate::saved_selections::INDEX {
            3
        } else if crate::saved_selections::is_entry_leaf(leaf) {
            4
        } else if leaf == crate::look::ENTRY {
            5
        } else if leaf.starts_with("composite/") {
            6
        } else if leaf.starts_with("meshmap-") && leaf.ends_with(".bin") {
            7
        } else if leaf == crate::distribution::IMPORTED_ORIGINAL {
            8
        } else {
            return None;
        };
        return Some(SET_ENTRIES[form]);
    }
    if let Some(rest) = name.strip_prefix("resources/") {
        let (content, ext) = rest.rsplit_once('.')?;
        return is_hash(content)
            .then(|| {
                RESOURCE_ENTRIES
                    .into_iter()
                    .find(|f| f.rsplit_once('.').is_some_and(|(_, e)| e == ext))
            })
            .flatten();
    }
    ROOT_ENTRIES.into_iter().find(|n| *n == name)
}
fn moves_into_set(n: &str) -> bool {
    ["document.utpaint", "selection.bin", "imported-original.psd"].contains(&n)
        || part_number(n).is_some()
        || n.starts_with("composite/")
        || n.starts_with("meshmap-") && n.ends_with(".bin")
}
/// 形式 7 以上の `ylp.json` の形式を、使っている機能に合わせる: 名前を付けて残した選択範囲のエントリが 1 つでもあれば 8、無ければ 7
/// （使うファイルだけが 8。旧形式は `upgraded` が 7 にする）。変わらなければ `ylp.json` のバイト列に触らない。
fn settle_format(files: &mut Files) -> Result<()> {
    let Some(blob) = files.get("ylp.json") else {
        return Ok(());
    };
    let mut info = json(&blob.bytes()?, 65536)?;
    let Some(format) = info.get("format").and_then(Value::as_i64) else {
        return Ok(());
    };
    if !(7..=i64::from(MAX_FORMAT)).contains(&format) {
        return Ok(());
    }
    let uses = files.keys().any(|n| {
        split_set(n).is_some_and(|(_, leaf)| crate::saved_selections::is_entry_leaf(leaf))
    });
    let want = if uses {
        i64::from(SAVED_SELECTIONS_FORMAT)
    } else {
        7
    };
    if format != want {
        info["format"] = Value::from(want);
        files.insert("ylp.json".into(), Blob::from(serde_json::to_vec(&info)?));
    }
    Ok(())
}
pub(crate) fn writer_json(w: &WriterInfo) -> Value {
    serde_json::json!({"app":w.app,"version":w.version,"unity":w.unity})
}
pub(crate) fn writer(v: &Value) -> Result<WriterInfo> {
    Ok(WriterInfo {
        app: text(v, "app", 1, 256)?.into(),
        version: text(v, "version", 1, 256)?.into(),
        unity: text(v, "unity", 1, 256)?.into(),
    })
}
/// エントリの並び（メモリの zip のものと、.ylp のもの）から中身を読む口。
pub(crate) trait EntryMap {
    fn entry(&self, name: &str) -> Option<Result<Arc<[u8]>>>;
}
impl EntryMap for crate::archive::Files {
    fn entry(&self, name: &str) -> Option<Result<Arc<[u8]>>> {
        self.get(name).map(|b| Ok(b.clone()))
    }
}
impl EntryMap for Files {
    fn entry(&self, name: &str) -> Option<Result<Arc<[u8]>>> {
        self.get(name).map(Blob::bytes)
    }
}
pub(crate) fn required(f: &impl EntryMap, n: &str) -> Result<Arc<[u8]>> {
    f.entry(n)
        .ok_or_else(|| Error::InvalidData(format!("エントリがありません: {n}")))?
}
pub(crate) fn text<'a>(v: &'a Value, k: &str, min: usize, max: usize) -> Result<&'a str> {
    let s = v
        .get(k)
        .and_then(Value::as_str)
        .ok_or_else(|| Error::InvalidData(format!("{k} が文字列ではありません")))?;
    check(
        (min..=max).contains(&s.encode_utf16().count()),
        format!("{k} の文字数が範囲外です"),
    )?;
    Ok(s)
}
pub(crate) fn label<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    let s = text(v, k, 1, 256)?;
    check(
        !s.trim().is_empty() && !s.chars().any(|c| c < ' ' || c == '\x7f'),
        "名前が空または制御文字を含んでいます",
    )?;
    Ok(s)
}
pub(crate) fn number(v: &Value, k: &str, min: i64, max: i64) -> Result<i64> {
    let n = v
        .get(k)
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::InvalidData(format!("{k} が整数ではありません")))?;
    check((min..=max).contains(&n), format!("{k} が範囲外です"))?;
    Ok(n)
}
fn id_text<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    let s = text(v, k, 36, 36)?;
    check(
        valid_id(s),
        format!("{k} は空でない小文字のGUIDである必要があります"),
    )?;
    Ok(s)
}
fn array<'a>(v: &'a Value, k: &str, min: usize, max: usize) -> Result<&'a Vec<Value>> {
    let a = v
        .get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| Error::InvalidData(format!("{k} が配列ではありません")))?;
    check(
        (min..=max).contains(&a.len()),
        format!("{k} の個数が範囲外です"),
    )?;
    Ok(a)
}
pub(crate) fn load_resources(
    files: &impl EntryMap,
    budget: &mut usize,
    depth: usize,
    notes: &mut Vec<Note>,
) -> Result<Vec<Resource>> {
    check(depth <= 8, "リソースの入れ子が深すぎます")?;
    let Some(b) = files.entry("resources.json") else {
        return Ok(Vec::new());
    };
    let root = json(&b?, 1024 * 1024)?;
    // 個数の上限は予算と同じ種類の断り（壊れたファイルとは別に、アセットがいっぱいだと言い分けられる）
    let list = array(&root, "resources", 0, usize::MAX)?;
    check_budget(
        list.len() <= crate::shelf::MAX_RESOURCES,
        crate::shelf::REFUSAL_RESOURCE_COUNT,
    )?;
    let mut ids = HashSet::new();
    let mut decoded = HashSet::new();
    let mut result = Vec::new();
    for v in list {
        let id = id_text(v, "id")?.to_string();
        check(ids.insert(id.clone()), "リソースのIDが重複しています")?;
        let name = label(v, "name")?.to_string();
        let kind = text(v, "kind", 1, 32)?.to_string();
        check(
            depth == 0 || kind == "image",
            "スマートリソースに画像以外のリソースがあります",
        )?;
        check(
            ["image", "smartMaterial", "smartMask", "brush", "material"].contains(&kind.as_str()),
            format!("未知のリソース種別です: {kind}"),
        )?;
        let content = text(v, "content", 64, 64)?.to_string();
        check(is_hash(&content), "リソースのハッシュが不正です")?;
        if let Some(origin) = v.get("origin").filter(|v| !v.is_null()) {
            validate_origin(origin)?;
        }
        let ext = match kind.as_str() {
            "image" => "png",
            "brush" => "ylbrush",
            _ => "ylsmart",
        };
        let entry = format!("resources/{content}.{ext}");
        let bytes = required(files, &entry)?;
        if kind == "image" {
            let w = number(v, "width", 1, 8192)? as u32;
            let h = number(v, "height", 1, 8192)? as u32;
            if let Some(c) = v.get("colorSpace").filter(|v| !v.is_null()) {
                check(
                    c.as_str()
                        .is_some_and(|s| ["srgb", "linear", "unspecified"].contains(&s)),
                    "未知の色空間です",
                )?;
            }
            if decoded.insert((content.clone(), w, h)) {
                let rgba = png_pixels(&bytes, w, h, budget)?;
                use sha2::{Digest, Sha256};
                let mut sha = Sha256::new();
                sha.update(b"YLPRGBA8");
                sha.update(w.to_le_bytes());
                sha.update(h.to_le_bytes());
                for row in rgba.chunks_exact(w as usize * 4).rev() {
                    sha.update(row);
                }
                check(
                    format!("{:x}", sha.finalize()) == content,
                    "画像の画素ハッシュが一致しません",
                )?;
            }
        } else {
            let len = number(v, "length", 1, MAX_ENTRY_BYTES as i64)? as usize;
            check(
                bytes.len() == len && hash(&bytes) == content,
                "リソースファイルの長さまたはSHA-256が一致しません",
            )?;
            if decoded.insert((format!("{content}:{kind}"), 0, 0)) {
                add_budget(budget, bytes.len())?;
                if kind == "brush" {
                    validate_brush(&bytes, budget, notes)?;
                } else {
                    let a = Archive::read_profile(
                        &bytes,
                        "application/x-yolupainter-smart",
                        "YOLUPAINTER-SMART-",
                        1,
                    )?;
                    let info = json(&required(&a.files, "smart.json")?, 65536)?;
                    number(&info, "format", 1, 1)?;
                    writer(&info["savedBy"])?;
                    label(&info, "name")?;
                    let actual = text(&info, "kind", 1, 32)?;
                    check(
                        actual
                            == if kind == "smartMask" {
                                "smartMask"
                            } else {
                                "smartMaterial"
                            },
                        "スマートリソースの種類が索引と一致しません",
                    )?;
                    let d = NativeDocument::read(&required(&a.files, "layers.utpaint")?)?;
                    check(
                        number(&info, "width", 1, 8192)? == d.width() as i64
                            && number(&info, "height", 1, 8192)? == d.height() as i64
                            && number(&info, "layers", 1, 2048)? == d.layer_count() as i64,
                        "スマートリソースの寸法・レイヤー数が一致しません",
                    )?;
                    validate_smart(&info, &d)?;
                    load_resources(&a.files, budget, depth + 1, notes)?;
                    notes.push(Note::SmartResourceKept(name.clone()));
                }
            }
        }
        result.push(Resource {
            id,
            kind,
            name,
            content,
            entry,
            metadata: v.clone(),
        });
    }
    Ok(result)
}
fn origin_text<'a>(v: &'a Value, k: &str, min: usize, max: usize) -> Result<&'a str> {
    let s = text(v, k, min, max)?;
    check(
        !s.chars().any(|c| c < ' ' || c == '\x7f'),
        "出どころに制御文字があります",
    )?;
    Ok(s)
}
fn validate_origin(v: &Value) -> Result<()> {
    match text(v, "type", 1, 32)? {
        "none" => {}
        "unityAsset" => {
            let g = text(v, "guid", 32, 32)?;
            check(
                g.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
                "アセットGUIDが不正です",
            )?;
            origin_text(v, "path", 1, 1024)?;
            if v.get("localFileID").is_some() {
                number(v, "localFileID", i64::MIN, i64::MAX)?;
            }
            if let Some(x) = v.get("stamp").filter(|v| !v.is_null()) {
                let _ = x;
                origin_text(v, "stamp", 0, 128)?;
            }
        }
        "file" | "library" => {
            let library = v["type"] == "library";
            let p = origin_text(v, if library { "file" } else { "path" }, 1, 1024)?;
            if library {
                check(
                    !p.contains('\\')
                        && !p.contains(':')
                        && p.split('/').all(|c| !c.is_empty() && c != ".." && c != "."),
                    "置き場の相対パスが不正です",
                )?;
            }
            check(
                is_hash(text(v, "sha256", 64, 64)?),
                "出どころのSHA-256が不正です",
            )?;
            number(v, "length", 0, i64::MAX)?;
        }
        "builtIn" => {
            let s = text(v, "key", 1, 64)?;
            check(
                s.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
                "内蔵リソースのキーが不正です",
            )?;
            number(v, "version", 1, i32::MAX as i64)?;
        }
        _ => return Err(Error::InvalidData("未知のリソース出どころです".into())),
    }
    Ok(())
}
fn add_budget(b: &mut usize, n: usize) -> Result<()> {
    *b = b
        .checked_add(n)
        .ok_or_else(|| Error::Budget("リソース予算超過です".into()))?;
    check_budget(*b <= MAX_TOTAL_BYTES, "復号リソースの768 MiB予算超過です")
}
/// 画像リソース（`kind == "image"` のもの）を効果の入力にする。プロジェクトとアセットが同じ道を使う（読み込みの検査と同じ: 寸法・ハッシュ・予算）。
/// `used` は、すでに復号して持っている画素のバイト数（呼び出しをまたいで通算するとき。この呼び出しの分と合わせて `limit` 以下）。
/// `limit` は復号した画素の合計に許すバイト数（`MAX_TOTAL_BYTES` を超えて広げられない）。
pub(crate) fn image_inputs_of<'a>(
    resources: impl Iterator<Item = &'a Resource>,
    files: &impl EntryMap,
    used: usize,
    limit: usize,
) -> Result<Vec<(yolu_core::ImageId, yolu_core::ImageInput)>> {
    let limit = limit.min(MAX_TOTAL_BYTES);
    let mut budget = used;
    let mut inputs = Vec::new();
    for r in resources.filter(|r| r.kind == "image") {
        let name = &r.name;
        let bytes = files
            .entry(&r.entry)
            .ok_or_else(|| Error::InvalidData(format!("画像「{name}」のPNGがありません")))??;
        let (w, h) = (
            number(&r.metadata, "width", 1, 8192)? as u32,
            number(&r.metadata, "height", 1, 8192)? as u32,
        );
        // 復号の前に、通算の予算で断る（確保する前に。寸法は索引の値で、PNG の寸法との一致は復号の中で確かめる）
        check_budget(
            budget
                .checked_add(w as usize * h as usize * 4)
                .is_some_and(|total| total <= limit),
            format!(
                "画像「{name}」: 復号した画像の{} MiB予算超過です",
                limit >> 20
            ),
        )?;
        let top_down = png_pixels(&bytes, w, h, &mut budget)
            .map_err(|e| Error::InvalidData(format!("画像「{name}」: {e}")))?;
        let mut pixels = Vec::with_capacity(top_down.len());
        for row in top_down.chunks_exact(w as usize * 4).rev() {
            pixels.extend_from_slice(row);
        }
        let space = match r.metadata.get("colorSpace").and_then(Value::as_str) {
            Some("srgb") => yolu_core::ImageColorSpace::Srgb,
            Some("linear") => yolu_core::ImageColorSpace::Linear,
            _ => yolu_core::ImageColorSpace::Unspecified,
        };
        let image = yolu_core::ImageInput::new(w, h, pixels, space)?;
        check(
            image.hash == r.content,
            format!("画像「{name}」の画素のハッシュが索引と一致しません"),
        )?;
        let id = u128::from_str_radix(&r.id.replace('-', ""), 16)
            .map_err(|_| Error::InvalidData(format!("画像「{name}」のIDが不正です")))?;
        inputs.push((yolu_core::ImageId(id), image));
    }
    Ok(inputs)
}
fn png_pixels(bytes: &[u8], w: u32, h: u32, budget: &mut usize) -> Result<Vec<u8>> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_limits(png::Limits {
        bytes: MAX_ENTRY_BYTES,
    });
    let mut r = decoder
        .read_info()
        .map_err(|e| Error::InvalidData(format!("PNGが不正です: {e}")))?;
    let i = r.info();
    check(
        i.width == w
            && i.height == h
            && i.color_type == png::ColorType::Rgba
            && i.bit_depth == png::BitDepth::Eight,
        "PNGは索引と同じ寸法のRGBA8である必要があります",
    )?;
    check(
        i.animation_control.is_none(),
        "アニメーションPNGは未対応です",
    )?;
    let len = w as usize * h as usize * 4;
    add_budget(budget, len)?;
    let mut out = vec![0; len];
    let frame = r
        .next_frame(&mut out)
        .map_err(|e| Error::InvalidData(format!("PNGの復号に失敗しました: {e}")))?;
    check(frame.buffer_size() == len, "PNGの復号長が不正です")?;
    r.finish()
        .map_err(|e| Error::InvalidData(format!("PNG終端が不正です: {e}")))?;
    Ok(out)
}
fn validate_brush(bytes: &[u8], budget: &mut usize, notes: &mut Vec<Note>) -> Result<()> {
    let a = Archive::read_profile(
        bytes,
        "application/x-yolupainter-brush",
        "YOLUPAINTER-BRUSH-",
        1,
    )?;
    let state = json(&required(&a.files, "state.json")?, 65536)?;
    let schema = state
        .get("schema")
        .and_then(Value::as_i64)
        .ok_or_else(|| Error::InvalidData("ブラシのschemaがありません".into()))?;
    check((1..=3).contains(&schema), "未知のブラシschemaです")?;
    for key in ["tipId", "textureId", "dualTipId"] {
        if let Some(v) = state.get(key) {
            check(v.as_str() == Some(""), "携帯ブラシに外部画像IDがあります")?;
        }
    }
    let mut tips = 0;
    for (name, b) in &a.files {
        if name.ends_with(".png") {
            if name.starts_with("tip-") {
                tips += 1;
            }
            let d = png::Decoder::new(Cursor::new(b.as_ref()));
            let r = d
                .read_info()
                .map_err(|e| Error::InvalidData(e.to_string()))?;
            let (w, h) = (r.info().width, r.info().height);
            check(
                (1..=2048).contains(&w) && (1..=2048).contains(&h),
                "ブラシの画像寸法が範囲外です",
            )?;
            png_pixels(b, w, h, budget)?;
        }
    }
    for i in 0..tips {
        check(
            a.files.contains_key(&format!("tip-{i}.png")),
            "ブラシの筆先番号が連続していません",
        )?;
    }
    notes.push(Note::BrushKept);
    Ok(())
}

// serde_json::Value 単体では重複キーを最後の値で上書きするため、ここで拒否する。
struct Strict(Value);
impl<'de> Deserialize<'de> for Strict {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Strict;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("重複キーのないJSON")
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> std::result::Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> std::result::Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> std::result::Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> std::result::Result<Strict, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Strict(Value::Number(n)))
                    .ok_or_else(|| E::custom("有限でない数値"))
            }
            fn visit_str<E: de::Error>(self, v: &str) -> std::result::Result<Strict, E> {
                Ok(Strict(v.into()))
            }
            fn visit_unit<E: de::Error>(self) -> std::result::Result<Strict, E> {
                Ok(Strict(Value::Null))
            }
            fn visit_none<E: de::Error>(self) -> std::result::Result<Strict, E> {
                self.visit_unit()
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Strict, A::Error> {
                let mut v = Vec::new();
                while let Some(Strict(x)) = a.next_element()? {
                    v.push(x);
                }
                Ok(Strict(Value::Array(v)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut a: A,
            ) -> std::result::Result<Strict, A::Error> {
                let mut v = Map::new();
                while let Some((k, Strict(x))) = a.next_entry::<String, Strict>()? {
                    if v.insert(k, x).is_some() {
                        return Err(de::Error::custom("重複したJSONキー"));
                    }
                }
                Ok(Strict(Value::Object(v)))
            }
        }
        d.deserialize_any(V)
    }
}
pub(crate) fn json(b: &[u8], max: usize) -> Result<Value> {
    check_budget(b.len() <= max, "JSONのバイト予算超過です")?;
    let Strict(v) = serde_json::from_slice(b)?;
    fn bounds(v: &Value, d: usize) -> bool {
        d <= 16
            && match v {
                Value::Array(a) => a.iter().all(|v| bounds(v, d + 1)),
                Value::Object(o) => o
                    .iter()
                    .all(|(k, v)| k.encode_utf16().count() <= 1024 && bounds(v, d + 1)),
                Value::String(s) => s.encode_utf16().count() <= 1024,
                _ => true,
            }
    }
    check(
        v.is_object() && bounds(&v, 0),
        "JSONの深さ・文字列長・最上位オブジェクトが不正です",
    )?;
    Ok(v)
}

pub(crate) fn validate_smart(info: &Value, d: &NativeDocument) -> Result<()> {
    let names = [
        "Color",
        "Roughness",
        "Metallic",
        "Height",
        "Normal",
        "Emission",
    ];
    let mask = info["kind"] == "smartMask";
    if mask {
        check(
            d.layer_count() == 1
                && d.field("layers[0].kind") == Some(&NativeValue::Int(1))
                && d.field("layers[0].fill_count") == Some(&NativeValue::Int(0))
                && d.field("layers[0].has_mask") == Some(&NativeValue::Bool(true))
                && d.field("layers[0].filters.count")
                    .is_none_or(|v| *v == NativeValue::Int(0)),
            "スマートマスクは値を持たない塗りつぶし 1 レイヤーとマスクが必要です",
        )?;
    }
    let mut enabled = HashSet::new();
    let mut stages = HashSet::new();
    for layer in 0..d.layer_count() {
        let prefix = format!("layers[{layer}]");
        if d.field(&format!("{prefix}.kind"))
            .is_none_or(|v| *v == NativeValue::Int(0))
        {
            let mut color_present = false;
            let mut color_enabled = true;
            for f in d.fields().iter().filter(|f| {
                f.path.starts_with(&format!("{prefix}.channels[")) && f.path.ends_with(".channel")
            }) {
                if f.value == NativeValue::Int(0) {
                    color_present = true;
                    color_enabled = d.field(&format!(
                        "{}.enabled",
                        f.path.strip_suffix(".channel").unwrap()
                    )) == Some(&NativeValue::Bool(true));
                }
            }
            if !color_present || color_enabled {
                enabled.insert(0);
            }
        }
    }
    for f in d.fields() {
        if f.path.ends_with(".has_surface_path") {
            check(
                f.value == NativeValue::Bool(false),
                "スマートリソースにはモデル上のパスを保存できません",
            )?;
        }
        if f.path.contains(".filters.items[") && f.path.ends_with(".generator.pin_count") {
            check(
                f.value == NativeValue::Int(0),
                "スマートリソースのジェネレーターにベイクの固定があります",
            )?;
            let p = f.path.strip_suffix(".generator.pin_count").unwrap();
            if let Some(NativeValue::Guid(id)) = d.field(&format!("{p}.id")) {
                stages.insert(crate::guid(id));
            }
        }
        if (f.path.contains(".fills[") || f.path.contains(".channels["))
            && f.path.ends_with(".channel")
            && !f.path.contains(".filters.")
        {
            if let NativeValue::Int(c) = f.value {
                let p = f.path.strip_suffix(".channel").unwrap();
                if f.path.contains(".adjustment.channels[")
                    || d.field(&format!("{p}.enabled")) == Some(&NativeValue::Bool(true))
                {
                    enabled.insert(c);
                }
            }
        }
    }
    if mask {
        enabled.clear();
    }
    let actual: Vec<_> = (0..6)
        .filter(|c| enabled.contains(c))
        .map(|c| Value::String(names[c as usize].into()))
        .collect();
    check(
        array(info, "channels", 0, 6)? == &actual,
        "スマートリソースのチャンネルが正本と一致しません",
    )?;
    if info.get("repin").is_some_and(|v| !v.is_null()) {
        let mut seen = HashSet::new();
        for v in array(info, "repin", 0, 65536)? {
            let id = v
                .as_str()
                .ok_or_else(|| Error::InvalidData("repinのIDが文字列ではありません".into()))?;
            check(
                valid_id(id) && seen.insert(id) && stages.contains(id),
                "repinは正本のジェネレーターIDを重複なく指定する必要があります",
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn brush(state: &str, names: &[&str]) -> Vec<u8> {
        let mut files =
            crate::archive::Files::from([("state.json".into(), Arc::from(state.as_bytes()))]);
        for n in names {
            files.insert(n.to_string(), Arc::from([0u8]));
        }
        Archive::build(
            files,
            1,
            "application/x-yolupainter-brush",
            "YOLUPAINTER-BRUSH-",
        )
        .unwrap()
        .to_bytes()
        .unwrap()
    }
    #[test]
    fn portable_brush_schemas_and_external_ids_are_checked() {
        for schema in 1..=3 {
            validate_brush(
                &brush(&format!("{{\"schema\":{schema}}}"), &[]),
                &mut 0,
                &mut Vec::new(),
            )
            .unwrap();
        }
        for state in [
            "{\"schema\":4}",
            "{\"schema\":0}",
            "{\"schema\":3,\"tipId\":\"external\"}",
            "{\"schema\":3,\"textureId\":null}",
        ] {
            assert!(validate_brush(&brush(state, &[]), &mut 0, &mut Vec::new()).is_err());
        }
    }
    #[test]
    fn portable_brush_invalid_png_is_refused() {
        assert!(validate_brush(
            &brush("{\"schema\":3}", &["tip-0.png"]),
            &mut 0,
            &mut Vec::new()
        )
        .is_err());
    }
    #[test]
    fn smart_metadata_channels_repins_and_model_paths_are_checked() {
        let archive = Archive::read(include_bytes!("../tests/fixtures/format5.ylp")).unwrap();
        let r = json(&archive.files["resources.json"], 1024 * 1024).unwrap();
        let material = r["resources"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["kind"] == "smartMaterial")
            .unwrap();
        let a = Archive::read_profile(
            &archive.files[&format!(
                "resources/{}.ylsmart",
                material["content"].as_str().unwrap()
            )],
            "application/x-yolupainter-smart",
            "YOLUPAINTER-SMART-",
            1,
        )
        .unwrap();
        let info = json(&a.files["smart.json"], 65536).unwrap();
        let doc = NativeDocument::read(&a.files["layers.utpaint"]).unwrap();
        validate_smart(&info, &doc).unwrap();
        let mut bad = info.clone();
        bad["channels"] = serde_json::json!(["Future"]);
        assert!(validate_smart(&bad, &doc).is_err());
        let mut bad = info;
        bad["repin"] = serde_json::json!(["aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"]);
        assert!(validate_smart(&bad, &doc).is_err());
    }
    #[test]
    fn decoded_resource_budget_is_bounded_without_allocating() {
        let mut budget = MAX_TOTAL_BYTES - 3;
        assert!(add_budget(&mut budget, 4).is_err());
    }
}
