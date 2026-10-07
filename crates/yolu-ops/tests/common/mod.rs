//! 試験の台: 小さな .ylp を作り、画面なしのホストで命令を JSON で当てる。
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use yolu_core::{Channel, Document, Rgba8};
use yolu_io::{
    composite_pngs, DocumentSource, MaterialRef, NativeDocument, NativeValue, Project, SaveTarget,
    SetSpec,
};
use yolu_ops::reply::LayerKindName;
use yolu_ops::{execute, parse_command, FileHost, OpError, OpHost, PathPolicy, Reply};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// 試験ごとの作業のフォルダ（落とすと消す）。
pub struct Fixture {
    pub dir: PathBuf,
}

impl Fixture {
    pub fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "yolu-ops-{}-{}-{name}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Fixture { dir }
    }
    pub fn path(&self, rel: &str) -> PathBuf {
        self.dir.join(rel)
    }
    pub fn policy(&self) -> PathPolicy {
        PathPolicy::new(&self.dir).unwrap()
    }
    /// 1 セット（既定の文書）の .ylp を書く。
    pub fn project(&self, file: &str) -> PathBuf {
        self.project_with(
            file,
            vec![Set::doc(
                "11111111-1111-4111-8111-111111111111",
                "Body",
                sample_document(),
            )],
        )
    }
    /// 複数のセットの .ylp を書く（最初のセットが今のセット）。
    pub fn project_with(&self, file: &str, sets: Vec<Set>) -> PathBuf {
        let path = self.path(file);
        let current = sets[0].id.clone();
        let specs: Vec<SetSpec> = sets.into_iter().map(Set::into_spec).collect();
        let project = Project::create(yolu_ops::writer(), &specs, &current).unwrap();
        SaveTarget::create(&path).unwrap().save(&project).unwrap();
        path
    }
    /// ファイルを開いたホスト（作業のフォルダはこのフォルダ）。
    pub fn host(&self, file: &str) -> FileHost {
        let mut host = FileHost::new(self.policy());
        host.open(&self.path(file), true).unwrap();
        host
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub struct Set {
    pub id: String,
    pub name: String,
    pub source: DocumentSource,
    pub composites: Vec<(Channel, Vec<u8>)>,
}

impl Set {
    pub fn doc(id: &str, name: &str, doc: Document) -> Set {
        let composites = composite_pngs(&doc).unwrap();
        Set {
            id: id.into(),
            name: name.into(),
            source: DocumentSource::from_core(Arc::new(doc)).unwrap(),
            composites,
        }
    }
    /// core が扱えない中身（使わない調整の値が既定でない）を持つ読むだけのセット。
    pub fn unsupported(id: &str, name: &str) -> Set {
        let mut doc = Document::new(16, 16).unwrap();
        doc.add_adjustment_layer("Adj", yolu_core::AdjustmentSettings::invert(), None, None)
            .unwrap();
        let native = NativeDocument::from_core(&doc).unwrap();
        let native = native
            .with_value("layers[0].adjustment.hue", NativeValue::Float(10.0))
            .unwrap();
        assert!(!native.core_issues().is_empty());
        Set {
            id: id.into(),
            name: name.into(),
            source: DocumentSource::from(native),
            composites: Vec::new(),
        }
    }
    fn into_spec(self) -> SetSpec {
        let material = MaterialRef::Material {
            name: self.name.clone(),
            asset: None,
        };
        SetSpec {
            id: self.id,
            name: self.name,
            material,
            document: Some(self.source),
            composites: self.composites,
        }
    }
}

/// 64×48 の文書: 下から Base（画素あり）・Tint（半透明の塗りつぶし）・Inner を入れた Group。
pub fn sample_document() -> Document {
    let mut doc = Document::new(64, 48).unwrap();
    let base = doc.add_layer("Base").unwrap();
    for y in 0..48 {
        for x in 0..64 {
            let c = Rgba8::new(
                (x * 4) as u8,
                (y * 5) as u8,
                90,
                if (x + y) % 7 == 0 { 0 } else { 255 },
            );
            doc.set_pixel(base, x, y, c).unwrap();
        }
    }
    doc.add_fill_layer(
        "Tint",
        &[(Channel::Color, Rgba8::new(40, 80, 160, 128))],
        None,
    )
    .unwrap();
    let inner = doc.add_layer("Inner").unwrap();
    doc.set_pixel(inner, 3, 4, Rgba8::new(255, 0, 0, 255))
        .unwrap();
    doc.group_layers(&[inner], "Group").unwrap();
    doc.clear_history().unwrap();
    doc
}

/// 命令の JSON を当てる（読み込み・版・引数の検査も通る）。
pub fn run(host: &mut dyn OpHost, command: Value) -> Result<Reply, OpError> {
    let command = parse_command(&command)?;
    execute(host, &command)
}

pub fn ok(host: &mut dyn OpHost, command: Value) -> Reply {
    let text = command.to_string();
    run(host, command).unwrap_or_else(|e| panic!("{text}: {} ({:?})", e.message.en, e.code))
}

pub fn err(host: &mut dyn OpHost, command: Value) -> OpError {
    let text = command.to_string();
    match run(host, command) {
        Ok(reply) => panic!("{text} が通った: {reply:?}"),
        Err(e) => e,
    }
}

/// レイヤーの ID を名前から（上から数えて最初）。
pub fn layer_id(host: &mut dyn OpHost, name: &str) -> String {
    let Reply::Set(info) = ok(host, json!({"command": "set.info"})) else {
        panic!()
    };
    info.layers
        .into_iter()
        .find(|l| l.name == name)
        .unwrap_or_else(|| panic!("レイヤー {name} が無い"))
        .id
}

/// 文書の見た目の全部（セットのレイヤーの並びと、レイヤーごとの全部）を JSON にして、状態の比べに使う。
pub fn state(host: &mut dyn OpHost) -> Value {
    let Reply::Set(info) = ok(host, json!({"command": "set.info"})) else {
        panic!()
    };
    let mut layers = Vec::new();
    for l in &info.layers {
        let Reply::Layer(mut detail) = ok(
            host,
            json!({"command": "layer.get", "args": {"layer": l.id}}),
        ) else {
            panic!()
        };
        // 値の無い塗りつぶしのチャンネルの「有効」の印は、合成に効かず .ylp に書かれない（core の決まり）ので、比べない
        if detail.summary.kind == LayerKindName::Fill {
            for c in &mut detail.channels {
                c.enabled = c.fill.is_some();
            }
        }
        layers.push(serde_json::to_value(detail).unwrap());
    }
    let mut set = serde_json::to_value(&info).unwrap();
    set.as_object_mut().unwrap().remove("unsaved");
    json!({"set": set, "layers": layers})
}

pub fn undo_count(host: &mut FileHost) -> usize {
    host.with_document(None, |d| d.undo_count()).unwrap()
}

pub fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}
