use crate::{
    archive::{split_set, Files},
    check, check_budget, hash, is_hash, valid_id, Archive, Error, NativeDocument, NativeValue, Result, Selection,
    MAX_ENTRY_BYTES, MAX_TOTAL_BYTES,
};
use serde::{
    de::{self, MapAccess, SeqAccess, Visitor},
    Deserialize, Deserializer,
};
use serde_json::{Map, Value};
use std::{collections::HashSet, fmt, io::Cursor, sync::Arc};

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
    pub document: NativeDocument,
    pub selection: Option<Selection>,
}
/// 書き手が作る・並べ直すセット（`Project::create`・`Project::with_sets`）。
#[derive(Clone, Debug)]
pub struct SetSpec {
    pub id: String,
    pub name: String,
    pub material: MaterialRef,
    /// 新しい正本。None なら元のプロジェクトの同じ ID のセットのエントリをバイト列のまま残す（新しいセットには要る）。
    pub document: Option<NativeDocument>,
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
                write!(f, "形式{format}のマテリアル参照を形式7へメモリ上で移行しました")
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
    original: Archive,
    files: Files,
    info: FormatInfo,
    sets: Vec<TextureSet>,
    current: String,
    resources: Vec<Resource>,
    notes: Vec<Note>,
    unknown: Vec<String>,
}
impl Project {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        Self::from_archive(Archive::read(bytes)?)
    }
    /// 全エントリ（`.ylp` のエントリ名 → 中身。`ylp.json` を含み、`mimetype` と manifest は含まない）から開く。復旧の世代
    /// （`GenerationStore`）から読んだエントリを、ZIP に詰め直さずに `Project` にする。検証は `read` と同じ。
    pub fn from_entries(entries: Files) -> Result<Self> {
        Self::from_archive(Archive::build(
            entries,
            3,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
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
        let mut budget = 0usize;
        let mut inputs = Vec::new();
        for r in self.resources.iter().filter(|r| r.kind == "image") {
            let name = &r.name;
            let bytes = self
                .files
                .get(&r.entry)
                .ok_or_else(|| Error::InvalidData(format!("画像「{name}」のPNGがありません")))?;
            let (w, h) = (
                number(&r.metadata, "width", 1, 8192)? as u32,
                number(&r.metadata, "height", 1, 8192)? as u32,
            );
            let top_down = png_pixels(bytes, w, h, &mut budget)
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
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }
    pub fn notes(&self) -> &[Note] {
        &self.notes
    }
    pub fn unknown_entries(&self) -> &[String] {
        &self.unknown
    }
    pub fn original_archive(&self) -> &Archive {
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
            json(b, 65536)?
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
        files.insert("ylp.json".into(), Arc::from(serde_json::to_vec(&info)?));
        Self::from_archive(Archive::build(
            files,
            3,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
    }
    /// 形式7の新しいプロジェクトを作る（セットは1〜64、どれも正本が要る）。書いたものは読み直して検証する。
    pub fn create(writer: WriterInfo, sets: &[SetSpec], current: &str) -> Result<Self> {
        let w = writer_json(&writer);
        let info = serde_json::json!({"format": 7, "savedBy": w, "createdBy": w});
        let mut files = Files::new();
        files.insert("ylp.json".into(), Arc::from(serde_json::to_vec(&info)?));
        let mut list = Vec::with_capacity(sets.len());
        for spec in sets {
            check(
                spec.document.is_some(),
                format!("新しいセット「{}」に正本がありません", spec.name),
            )?;
            list.push(set_json(None, spec));
            put_set_entries(&mut files, spec)?;
        }
        let project = serde_json::json!({"sets": list, "current": current});
        files.insert(
            "project.json".into(),
            Arc::from(serde_json::to_vec(&project)?),
        );
        Self::from_archive(Archive::build(
            files,
            3,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
    }
    /// 形式7のセットの並び・名前・マテリアル参照・現在のセットを置き換え、正本と合成を差し替える。旧形式は先にupgradedで
    /// 明示的に移行する。元のセットは全部が並びに要る（セットを消すのは別の口で、黙って消さない）。並びに無かったIDは新しい
    /// セットで、正本が要る。各セットの知らないJSONキー、選択範囲・メッシュマップ・PSD原本、根のほかのエントリは残す。
    /// `savedBy` は writer にする。
    pub fn with_sets(&self, writer: WriterInfo, sets: &[SetSpec], current: &str) -> Result<Self> {
        check(
            self.info.format == 7,
            "セットを並べ直す前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        let mut project = json(required(&files, "project.json")?, 65536)?;
        let old: Vec<Value> = project["sets"].as_array().cloned().unwrap_or_default();
        for set in &self.sets {
            check(
                sets.iter().any(|s| s.id == set.id),
                format!(
                    "セット「{}」が並びにありません（消すのは別の口です）",
                    set.name
                ),
            )?;
        }
        let mut list = Vec::with_capacity(sets.len());
        for spec in sets {
            let previous = old
                .iter()
                .find(|v| v["id"].as_str() == Some(spec.id.as_str()));
            check(
                previous.is_some() || spec.document.is_some(),
                format!("新しいセット「{}」に正本がありません", spec.name),
            )?;
            list.push(set_json(previous, spec));
            put_set_entries(&mut files, spec)?;
        }
        project["sets"] = Value::Array(list);
        project["current"] = Value::from(current);
        files.insert(
            "project.json".into(),
            Arc::from(serde_json::to_vec(&project)?),
        );
        let mut info = json(required(&files, "ylp.json")?, 65536)?;
        info["savedBy"] = writer_json(&writer);
        files.insert("ylp.json".into(), Arc::from(serde_json::to_vec(&info)?));
        Self::from_archive(Archive::build(
            files,
            self.original.level,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
    }
    /// 既存セットの正本だけを置き換える。選択範囲・リソースを含めて再検証する。
    pub fn with_document(&self, set_id: &str, doc: &NativeDocument) -> Result<Self> {
        check(
            self.sets.iter().any(|s| s.id == set_id),
            "セットがありません",
        )?;
        let name = if self.info.format < 3 {
            "document.utpaint".into()
        } else {
            format!("sets/{set_id}/document.utpaint")
        };
        let mut files = self.original.files.clone();
        files.insert(name, Arc::from(doc.to_bytes()));
        Self::from_archive(Archive::build(
            files,
            self.original.level,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
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
                files.insert(name, Arc::from(s.to_bytes()));
            }
            None => {
                files.remove(&name);
            }
        }
        Self::from_archive(Archive::build(
            files,
            self.original.level,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
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
            .map(|bytes| {
                let map = crate::mesh_map::read_with_limit(bytes, max_bytes)?;
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
            self.info.format == 7,
            "メッシュマップを書く前にupgradedで形式7へ移行してください",
        )?;
        let name = self.mesh_map_entry(set_id, map.kind())?;
        let mut files = self.original.files.clone();
        files.insert(name, Arc::from(crate::mesh_map::write(map)?));
        Self::from_archive(Archive::build(
            files,
            self.original.level,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
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
            self.info.format == 7,
            "マテリアル参照を変える前にupgradedで形式7へ移行してください",
        )?;
        let mut files = self.original.files.clone();
        let mut project = json(required(&files, "project.json")?, 65536)?;
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
            Arc::from(serde_json::to_vec(&project)?),
        );
        Self::from_archive(Archive::build(
            files,
            self.original.level,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
    }
    pub(crate) fn from_archive(original: Archive) -> Result<Self> {
        let info = if let Some(b) = original.files.get("ylp.json") {
            let root = json(b, 65536)?;
            let format = number(&root, "format", 2, i32::MAX as i64)? as i32;
            let saved = writer(&root["savedBy"])?;
            let created = if root.get("createdBy").is_some_and(|v| !v.is_null()) {
                Some(writer(&root["createdBy"])?)
            } else {
                None
            };
            if format > 7 {
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
            let doc = NativeDocument::read(
                files
                    .get("document.utpaint")
                    .ok_or_else(|| Error::InvalidData("旧形式の正本がありません".into()))?,
            )?;
            let mut slot = 0;
            if let Some(b) = files.get("view.json") {
                match json(b, 65536).and_then(|j| {
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
            files.insert("project.json".into(), Arc::from(project.into_bytes()));
            notes.push(Note::Migrated { format: info.format });
        }
        let mut root = json(required(&files, "project.json")?, 65536)?;
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
            files.insert("project.json".into(), Arc::from(serde_json::to_vec(&root)?));
            notes.push(Note::MaterialRefsMigrated { format: info.format });
        }
        let list = array(&root, "sets", 1, 64)?;
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
            let document =
                NativeDocument::read(required(&files, &format!("{prefix}document.utpaint"))?)?;
            let selection = files
                .get(&format!("{prefix}selection.bin"))
                .map(|b| Selection::read(b, &document))
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
            let known = if let Some((id, leaf)) = split_set(n) {
                ids.contains(id) && moves_into_set(leaf)
            } else if n.starts_with("resources/") {
                resource_entries.contains(n.as_str())
            } else {
                [
                    "project.json",
                    "resources.json",
                    "view.json",
                    "brush.json",
                    "thumbnail.png",
                    "model.json",
                ]
                .contains(&n.as_str())
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
fn put_set_entries(files: &mut Files, spec: &SetSpec) -> Result<()> {
    let Some(doc) = &spec.document else {
        return Ok(());
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
    files.retain(|n, _| !n.starts_with(&format!("{prefix}composite/")));
    files.insert(
        format!("{prefix}document.utpaint"),
        Arc::from(doc.to_bytes()),
    );
    for (name, png) in composites {
        files.insert(
            format!("{prefix}composite/{name}.png"),
            Arc::from(png.as_slice()),
        );
    }
    Ok(())
}
fn moves_into_set(n: &str) -> bool {
    ["document.utpaint", "selection.bin", "imported-original.psd"].contains(&n)
        || n.starts_with("composite/")
        || n.starts_with("meshmap-") && n.ends_with(".bin")
}
fn writer_json(w: &WriterInfo) -> Value {
    serde_json::json!({"app":w.app,"version":w.version,"unity":w.unity})
}
pub(crate) fn writer(v: &Value) -> Result<WriterInfo> {
    Ok(WriterInfo {
        app: text(v, "app", 1, 256)?.into(),
        version: text(v, "version", 1, 256)?.into(),
        unity: text(v, "unity", 1, 256)?.into(),
    })
}
pub(crate) fn required<'a>(f: &'a Files, n: &str) -> Result<&'a [u8]> {
    f.get(n)
        .map(|b| b.as_ref())
        .ok_or_else(|| Error::InvalidData(format!("エントリがありません: {n}")))
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
    files: &Files,
    budget: &mut usize,
    depth: usize,
    notes: &mut Vec<Note>,
) -> Result<Vec<Resource>> {
    check(depth <= 8, "リソースの入れ子が深すぎます")?;
    let Some(b) = files.get("resources.json") else {
        return Ok(Vec::new());
    };
    let root = json(b, 1024 * 1024)?;
    // 個数の上限は予算と同じ種類の断り（壊れたファイルとは別に、棚がいっぱいだと言い分けられる）
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
                let rgba = png_pixels(bytes, w, h, budget)?;
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
                bytes.len() == len && hash(bytes) == content,
                "リソースファイルの長さまたはSHA-256が一致しません",
            )?;
            if decoded.insert((format!("{content}:{kind}"), 0, 0)) {
                add_budget(budget, bytes.len())?;
                if kind == "brush" {
                    validate_brush(bytes, budget, notes)?;
                } else {
                    let a = Archive::read_profile(
                        bytes,
                        "application/x-yolupainter-smart",
                        "YOLUPAINTER-SMART-",
                        1,
                    )?;
                    let info = json(required(&a.files, "smart.json")?, 65536)?;
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
                    let d = NativeDocument::read(required(&a.files, "layers.utpaint")?)?;
                    check(
                        number(&info, "width", 1, 8192)? == d.width() as i64
                            && number(&info, "height", 1, 8192)? == d.height() as i64
                            && number(&info, "layers", 1, 2048)? == d.layer_count() as i64,
                        "スマートリソースの寸法・層数が一致しません",
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
    let state = json(required(&a.files, "state.json")?, 65536)?;
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
            let r = d.read_info().map_err(|e| Error::InvalidData(e.to_string()))?;
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
            "スマートマスクは値を持たない塗りつぶし1層とマスクが必要です",
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
                "スマートリソースのGeneratorにベイクの固定があります",
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
                "repinは正本のGenerator IDを重複なく指定する必要があります",
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn brush(state: &str, names: &[&str]) -> Vec<u8> {
        let mut files = Files::from([("state.json".into(), Arc::from(state.as_bytes()))]);
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
