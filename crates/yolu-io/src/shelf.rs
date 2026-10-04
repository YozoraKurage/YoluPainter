//! プロジェクト内の素材。外部の参照先を開かず、検証した埋め込みの写しを保持する。
use crate::{
    archive::Files, check, project, Archive, NativeDocument, NativeValue, Project, Resource,
    Result, MAX_TOTAL_BYTES,
};
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Image,
    SmartMaterial,
    SmartMask,
    Brush,
    Material,
}
impl ResourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::SmartMaterial => "smartMaterial",
            Self::SmartMask => "smartMask",
            Self::Brush => "brush",
            Self::Material => "material",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Shelf {
    files: Files,
    resources: Vec<Resource>,
    budget: u64,
    used: u64,
}
impl Shelf {
    pub fn new(budget: u64) -> Self {
        Self {
            files: Files::new(),
            resources: vec![],
            budget,
            used: 0,
        }
    }
    pub fn read(files: &Files, budget: u64) -> Result<Self> {
        let resources = project::load_resources(files, &mut 0, 0, &mut vec![])?;
        let mut held = Files::new();
        if let Some(b) = files.get("resources.json") {
            held.insert("resources.json".into(), b.clone());
        }
        for r in &resources {
            held.insert(r.entry.clone(), files[&r.entry].clone());
        }
        let used = memory(&resources, &held)?;
        // 読み込みは予算以下への切り捨てをしない。追加時に予算を守る。
        Ok(Self {
            files: held,
            resources,
            budget,
            used,
        })
    }
    pub fn resources(&self) -> &[Resource] {
        &self.resources
    }
    pub fn used_bytes(&self) -> u64 {
        self.used
    }
    pub fn budget_bytes(&self) -> u64 {
        self.budget
    }
    pub fn set_budget_bytes(&mut self, budget: u64) {
        self.budget = budget;
    }
    pub fn entries(&self) -> &Files {
        &self.files
    }
    pub fn content_bytes(&self, id: &str) -> Option<&[u8]> {
        self.resources
            .iter()
            .find(|r| r.id == id)
            .map(|r| self.files[&r.entry].as_ref())
    }
    /// 索引の1項目と写しを追加。同種・同内容は既存の ID を返す。拒否では何も変えない。
    pub fn add(&mut self, metadata: Value, bytes: &[u8]) -> Result<String> {
        let content = project::text(&metadata, "content", 64, 64)?;
        let kind = project::text(&metadata, "kind", 1, 32)?;
        let ext = match kind {
            "image" => "png",
            "brush" => "ylbrush",
            _ => "ylsmart",
        };
        let mut one = Files::new();
        one.insert(
            "resources.json".into(),
            Arc::from(serde_json::to_vec(
                &json!({"resources":[metadata.clone()]}),
            )?),
        );
        one.insert(format!("resources/{content}.{ext}"), Arc::from(bytes));
        let checked = Self::read(&one, self.budget)?;
        let item = &checked.resources[0];
        if let Some(old) = self
            .resources
            .iter()
            .find(|r| r.kind == item.kind && r.content == item.content)
        {
            return Ok(old.id.clone());
        }
        check(
            !self.resources.iter().any(|r| r.id == item.id),
            "リソースのIDが重複しています",
        )?;
        let mut resources = self.resources.clone();
        resources.push(item.clone());
        let mut files = self.files.clone();
        files.insert(item.entry.clone(), Arc::from(bytes));
        files.insert("resources.json".into(), Arc::from(write_index(&resources)?));
        let next = Self::read(&files, self.budget)?;
        check(next.used <= self.budget, "素材のメモリ予算超過です")?;
        check(
            files.values().map(|b| b.len()).sum::<usize>() <= MAX_TOTAL_BYTES,
            "素材のアーカイブ予算超過です",
        )?;
        let id = item.id.clone();
        *self = next;
        Ok(id)
    }
    pub fn remove(&mut self, id: &str) -> Result<bool> {
        let mut resources = self.resources.clone();
        resources.retain(|r| r.id != id);
        if resources.len() == self.resources.len() {
            return Ok(false);
        }
        let mut files = self.files.clone();
        files.retain(|n, _| n == "resources.json" || resources.iter().any(|r| r.entry == *n));
        // リソースの無い棚は resources.json を持たない（C# の ResourceIndex.AddTo と同じ。最初の 1 件を足すときに作る）
        if resources.is_empty() {
            files.remove("resources.json");
        } else {
            files.insert("resources.json".into(), Arc::from(write_index(&resources)?));
        }
        *self = Self::read(&files, self.budget)?;
        Ok(true)
    }
    /// C# ResourceIndex.Write と同じ索引。未知のキーを含む原本の保存には entries を使う。
    pub fn canonical_index(&self) -> Result<Vec<u8>> {
        write_index(&self.resources)
    }
}
impl Project {
    pub fn shelf(&self, budget: u64) -> Result<Shelf> {
        Shelf::read(self.migrated_entries(), budget)
    }
    /// セットと未知のエントリを保ち、棚を差し替えた新しいプロジェクトを検証して返す。形式7へ上げ（`upgraded` と同じ）、
    /// `savedBy` は書き手（最後に保存したアプリ）にする。
    pub fn with_shelf(&self, shelf: &Shelf, writer: crate::WriterInfo) -> Result<Self> {
        let mut files = self.migrated_entries().clone();
        for r in self.resources() {
            files.remove(&r.entry);
        }
        files.remove("resources.json");
        files.extend(shelf.files.clone());
        let upgraded = self.upgraded(writer)?;
        files.insert(
            "ylp.json".into(),
            upgraded.original_archive().entries()["ylp.json"].clone(),
        );
        Self::from_archive(Archive::build(
            files,
            3,
            "application/x-yolupainter",
            "YOLUPAINTER-YLP-",
        )?)
    }
}
pub(crate) fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c < ' ' => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
fn origin(v: &Value) -> String {
    let q = |k: &str| quote(v[k].as_str().unwrap_or(""));
    match v["type"].as_str().unwrap_or("none") {
        "unityAsset"=>format!("{{ \"type\": \"unityAsset\", \"guid\": {}, \"localFileID\": {}, \"path\": {}, \"stamp\": {}, \"readThroughGpu\": {} }}",q("guid"),v["localFileID"].as_i64().unwrap_or(0),q("path"),q("stamp"),v["readThroughGpu"].as_bool().unwrap_or(false)),
        "file"|"library"=>{let ty=v["type"].as_str().unwrap();let key=if ty=="file" {"path"} else {"file"};format!("{{ \"type\": \"{ty}\", \"{key}\": {}, \"sha256\": {}, \"length\": {} }}",q(key),q("sha256"),v["length"])},
        "builtIn"=>format!("{{ \"type\": \"builtIn\", \"key\": {}, \"version\": {} }}",q("key"),v["version"]),
        _=>"{ \"type\": \"none\" }".into()
    }
}
fn write_index(resources: &[Resource]) -> Result<Vec<u8>> {
    let mut s = String::from("{\n  \"resources\": [");
    for (i, r) in resources.iter().enumerate() {
        s.push_str(if i == 0 { "\n" } else { ",\n" });
        s.push_str(&format!(
            "    {{ \"id\": {}, \"kind\": {}, \"name\": {}, \"content\": {}",
            quote(&r.id),
            quote(&r.kind),
            quote(&r.name),
            quote(&r.content)
        ));
        let v = &r.metadata;
        if r.kind == "image" {
            s.push_str(&format!(
                ", \"width\": {}, \"height\": {}, \"colorSpace\": {}",
                v["width"],
                v["height"],
                quote(v["colorSpace"].as_str().unwrap_or("unspecified"))
            ));
        } else {
            s.push_str(&format!(", \"length\": {}", v["length"]));
        }
        s.push_str(&format!(",\n      \"origin\": {} }}", origin(&v["origin"])));
    }
    s.push_str(if resources.is_empty() {
        "]\n}\n"
    } else {
        "\n  ]\n}\n"
    });
    check(s.len() <= 1024 * 1024, "resources.json の予算超過です")?;
    Ok(s.into_bytes())
}
fn memory(resources: &[Resource], files: &Files) -> Result<u64> {
    let mut seen = HashSet::new();
    let mut bytes = 0;
    for r in resources {
        if r.kind == "image" {
            if seen.insert(r.content.clone()) {
                bytes += r.metadata["width"].as_u64().unwrap()
                    * r.metadata["height"].as_u64().unwrap()
                    * 4;
            }
            continue;
        }
        let b = &files[&r.entry];
        bytes += b.len() as u64;
        if r.kind == "brush" {
            let a = Archive::read_profile(
                b,
                "application/x-yolupainter-brush",
                "YOLUPAINTER-BRUSH-",
                1,
            )?;
            for (name, b) in a.entries() {
                if name.ends_with(".png") {
                    let reader = png::Decoder::new(std::io::Cursor::new(b.as_ref()))
                        .read_info()
                        .map_err(|e| crate::Error(e.to_string()))?;
                    bytes += reader.info().width as u64 * reader.info().height as u64;
                }
            }
        } else {
            let a = Archive::read_profile(
                b,
                "application/x-yolupainter-smart",
                "YOLUPAINTER-SMART-",
                1,
            )?;
            let native = NativeDocument::read(&a.entries()["layers.utpaint"])?;
            for f in native.fields() {
                if f.path.contains(".tiles[") && f.path.ends_with(".rgba") {
                    if let NativeValue::Bytes(b) = &f.value {
                        bytes += if b.as_chunks::<4>().0.iter().all(|p| p == &b[..4]) {
                            if b[..4] == [0; 4] {
                                0
                            } else {
                                4
                            }
                        } else {
                            b.len() as u64
                        };
                    }
                }
            }
            let nested = project::load_resources(a.entries(), &mut 0, 1, &mut vec![])?;
            bytes += memory(&nested, a.entries())?;
        }
    }
    Ok(bytes)
}
/// 画像の正本ハッシュ。PNG の圧縮方法に依存せず、透明画素の RGB も含む。
pub fn image_hash(rgba: &[u8], width: u32, height: u32) -> Result<String> {
    check(
        (1..=8192).contains(&width) && (1..=8192).contains(&height),
        "画像寸法は1〜8192です",
    )?;
    check(
        rgba.len() as u64 == width as u64 * height as u64 * 4,
        "RGBA8 の長さが不正です",
    )?;
    use sha2::{Digest, Sha256};
    let mut sha = Sha256::new();
    sha.update(b"YLPRGBA8");
    sha.update(width.to_le_bytes());
    sha.update(height.to_le_bytes());
    sha.update(rgba);
    Ok(format!("{:x}", sha.finalize()))
}
impl Shelf {
    /// 左下原点・straight RGBA8 の画像を追加する。origin は参照の記録だけで、読み込みに使わない。
    #[allow(clippy::too_many_arguments)]
    pub fn add_image(
        &mut self,
        id: &str,
        name: &str,
        rgba: &[u8],
        width: u32,
        height: u32,
        color_space: &str,
        origin: Value,
    ) -> Result<String> {
        let content = image_hash(rgba, width, height)?;
        let metadata = json!({"id":id,"kind":"image","name":name,"content":content,"width":width,"height":height,"colorSpace":color_space,"origin":origin});
        let png = crate::composite_png::encode(rgba, width, height)?;
        self.add(metadata, &png)
    }
    pub fn add_file(
        &mut self,
        id: &str,
        name: &str,
        kind: ResourceKind,
        bytes: &[u8],
        origin: Value,
    ) -> Result<String> {
        check(
            kind != ResourceKind::Image,
            "画像は素材のファイルとして追加できません",
        )?;
        self.add(json!({"id":id,"kind":kind.as_str(),"name":name,"content":crate::hash(bytes),"length":bytes.len(),"origin":origin}),bytes)
    }
}
