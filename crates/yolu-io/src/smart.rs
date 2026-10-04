//! .ylsmart の検証と原本の保持。core が扱えない正本の属性は変換時に拒否する。
use crate::{
    archive::Files,
    check, project,
    shelf::{quote, Shelf},
    Archive, NativeDocument, Result, WriterInfo,
};
use serde_json::{json, Value};
use std::sync::Arc;
use yolu_core::{
    smart::{SmartKind, SmartMaterial},
    Document,
};
/// 編集用に開けない・保存できない理由の文言。アプリが文言から「置けない理由」を見分けるので、文言はここにだけ書く。
pub const REFUSAL_IMAGES: &str = "画像を含む素材は編集用に開けません";
pub const REFUSAL_GENERATORS: &str = "Generator の再固定を持つ素材は編集用に開けません";
pub const REFUSAL_USER_CHANNELS: &str = ".ylsmart 形式1は標準チャンネルだけです";
pub const REFUSAL_RUST_GENERATORS: &str =
    ".ylsmart 形式1は Unity 版にもある Generator の種類だけです（ノイズ・グランジは入れられません）";
const MIME: &str = "application/x-yolupainter-smart";
const PREFIX: &str = "YOLUPAINTER-SMART-";
#[derive(Clone, Debug)]
pub struct SmartFile {
    bytes: Arc<[u8]>,
    archive: Archive,
    info: Value,
    fragment: NativeDocument,
}
impl SmartFile {
    pub fn from_entries(files: Files) -> Result<Self> {
        Self::read(&Archive::build(files, 1, MIME, PREFIX)?.to_bytes()?)
    }
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let archive = Archive::read_profile(bytes, MIME, PREFIX, 1)?;
        let info = project::json(project::required(archive.entries(), "smart.json")?, 65536)?;
        project::number(&info, "format", 1, 1)?;
        project::writer(&info["savedBy"])?;
        project::label(&info, "name")?;
        check(
            matches!(info["kind"].as_str(), Some("smartMaterial" | "smartMask")),
            "未知のスマート素材の種類です",
        )?;
        let fragment =
            NativeDocument::read(project::required(archive.entries(), "layers.utpaint")?)?;
        check(
            project::number(&info, "width", 1, 8192)? == fragment.width() as i64
                && project::number(&info, "height", 1, 8192)? == fragment.height() as i64
                && project::number(&info, "layers", 1, 2048)? == fragment.layer_count() as i64,
            "スマート素材の寸法・層数が一致しません",
        )?;
        project::validate_smart(&info, &fragment)?;
        project::load_resources(archive.entries(), &mut 0, 1, &mut vec![])?;
        Ok(Self {
            bytes: Arc::from(bytes),
            archive,
            info,
            fragment,
        })
    }
    /// 原本の ZIP・圧縮・未知のエントリも含め全バイトを保つ。
    pub fn file_bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn entries(&self) -> &Files {
        self.archive.entries()
    }
    pub fn info(&self) -> &Value {
        &self.info
    }
    pub fn fragment(&self) -> &NativeDocument {
        &self.fragment
    }
    pub fn kind(&self) -> SmartKind {
        if self.info["kind"] == "smartMask" {
            SmartKind::Mask
        } else {
            SmartKind::Material
        }
    }
    /// 編集用の core の素材にする。画素の予算は付けない: 配置先の文書の予算は配置のときに別に検査され、Unity 版が保存して読める
    /// 大きさの素材を、読み込みで先に断らない。断片の大きさは .ylsmart のエントリの上限が抑える。
    pub fn to_core(&self) -> Result<SmartMaterial> {
        self.to_core_within(u64::MAX)
    }
    /// 画素の予算を指定して core の素材にする。超えたら、どの層のどのタイルで断ったかを添えて断る。
    pub fn to_core_within(&self, budget: u64) -> Result<SmartMaterial> {
        check(
            self.archive
                .entries()
                .get("resources.json")
                .is_none_or(|bytes| {
                    project::json(bytes, 1024 * 1024)
                        .ok()
                        .and_then(|v| v["resources"].as_array().map(Vec::is_empty))
                        .unwrap_or(false)
                }),
            REFUSAL_IMAGES,
        )?;
        check(
            self.info["repin"].as_array().is_none_or(|a| a.is_empty()),
            REFUSAL_GENERATORS,
        )?;
        let doc = self.fragment.to_core_within(Some(budget))?;
        Ok(doc.into_smart_material(self.kind(), self.info["name"].as_str().expect("検証済み"))?)
    }
    pub fn from_core(material: &SmartMaterial, writer: &WriterInfo) -> Result<Self> {
        let doc: Document = material.fragment_document()?;
        // 断片が持つチャンネル定義は、層が何かを持つものだけ。ユーザーチャンネルを使う素材は、Unity 版が読めない版で書くことになる
        check(
            doc.channels().iter().all(|c| c.is_standard()),
            REFUSAL_USER_CHANNELS,
        )?;
        // ノイズ・グランジ（Rust 版だけの種類）は正本の版 23 になり、Unity 版が読めない
        check(
            !crate::core_bridge::uses_rust_only_generators(&doc),
            REFUSAL_RUST_GENERATORS,
        )?;
        let native = NativeDocument::from_core(&doc)?;
        let names = [
            "Color",
            "Roughness",
            "Metallic",
            "Height",
            "Normal",
            "Emission",
        ];
        let channels = material
            .channels()
            .iter()
            .map(|c| quote(names[c.index()]))
            .collect::<Vec<_>>()
            .join(", ");
        let kind = if material.kind() == SmartKind::Mask {
            "smartMask"
        } else {
            "smartMaterial"
        };
        let info=format!("{{\n  \"format\": 1,\n  \"kind\": \"{kind}\",\n  \"name\": {},\n  \"width\": {}, \"height\": {},\n  \"layers\": {},\n  \"channels\": [{channels}],\n  \"repin\": [],\n  \"savedBy\": {{ \"app\": {}, \"version\": {}, \"unity\": {} }}\n}}\n",quote(material.name()),material.width(),material.height(),material.layers().len(),quote(&writer.app),quote(&writer.version),quote(&writer.unity));
        let files = Files::from([
            ("smart.json".into(), Arc::from(info.into_bytes())),
            ("layers.utpaint".into(), Arc::from(native.to_bytes())),
        ]);
        Self::read(&Archive::build(files, 1, MIME, PREFIX)?.to_bytes()?)
    }
    /// 埋め込み画像を棚へまとめて追加。途中の拒否では棚を変更しない。
    pub fn add_images_to(
        &self,
        shelf: &mut Shelf,
    ) -> Result<std::collections::BTreeMap<String, String>> {
        let nested = Shelf::read(self.entries(), shelf.budget_bytes())?;
        let mut next = shelf.clone();
        let mut map = std::collections::BTreeMap::new();
        for r in nested.resources() {
            let mut metadata = r.metadata.clone();
            if next
                .resources()
                .iter()
                .any(|old| old.id == r.id && old.content != r.content)
            {
                // 文書の UUID 生成を使い、衝突する元 ID は書き換えて返す。
                let d = Document::new(1, 1)?;
                let id = d.id().to_be_bytes();
                let hex = id.iter().map(|b| format!("{b:02x}")).collect::<String>();
                metadata["id"] = json!(format!(
                    "{}-{}-{}-{}-{}",
                    &hex[..8],
                    &hex[8..12],
                    &hex[12..16],
                    &hex[16..20],
                    &hex[20..]
                ));
            }
            map.insert(
                r.id.clone(),
                next.add(metadata, nested.content_bytes(&r.id).expect("検証済み"))?,
            );
        }
        *shelf = next;
        Ok(map)
    }
}
