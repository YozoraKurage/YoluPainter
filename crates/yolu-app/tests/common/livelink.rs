//! Live Link のファイルの受け渡しの試験のツール: 試しの受け渡しのフォルダに、試しの FBX（yolu-model の試験と同じ ASCII の腕）と絵を
//! 置き、頼みの JSON を Unity と同じ手順（`.tmp` に書いてから名前を変える）で `inbox/` に置き、`outbox/` の返事を読む。実のデータは使わない。
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use yolu_protocol::files::{write_replacing, Folder, Reply};

#[path = "../../../yolu-model/tests/common/fbx_ascii.rs"]
pub mod fbx_ascii;

/// 試しの受け渡しのフォルダと、FBX・絵の置き場。
pub struct Exchange {
    /// 試験の一時のフォルダ（終わると消える）。
    pub dir: PathBuf,
    /// 受け渡しのフォルダ（`LiveLink::set_folder` に渡す）。
    pub root: PathBuf,
}

impl Exchange {
    pub fn new(tag: &str) -> Exchange {
        let dir = super::tmp::test_dir(&format!("ll-{tag}"));
        Exchange {
            root: dir.join("LiveLink"),
            dir,
        }
    }

    /// Unity の側の手で開いたフォルダ（無ければ作る）。
    pub fn folder(&self) -> Folder {
        Folder::open(&self.root).expect("受け渡しのフォルダ")
    }

    /// 頼みを `inbox/<id>.json` に置く（`.tmp` から名前を変える）。
    pub fn put(&self, request: &Value) {
        let id = request["id"].as_str().expect("id");
        self.put_bytes(&format!("{id}.json"), &serde_json::to_vec(request).unwrap());
    }

    /// バイト列を `inbox/<名前>` に置く（壊れた頼みの試験）。
    pub fn put_bytes(&self, name: &str, bytes: &[u8]) {
        let f = self.folder();
        write_replacing(&f.inbox().join(name), bytes).unwrap();
    }

    /// 返事を全部読む（読んだ返事は消す。Unity の側と同じ）。
    pub fn take_replies(&self) -> Vec<Reply> {
        let f = self.folder();
        let mut out = Vec::new();
        for path in f.replies().unwrap() {
            let reply: Reply = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
            std::fs::remove_file(&path).unwrap();
            out.push(reply);
        }
        out
    }

    /// 試しの腕の FBX を書く（`fbx_ascii::arm_scene`。Unity が取り込める形の Definitions の節つき）。
    pub fn write_arm(&self, name: &str) -> PathBuf {
        self.write_scene(name, &fbx_ascii::arm_scene())
    }

    pub fn write_scene(&self, name: &str, scene: &fbx_ascii::Scene) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, scene.to_ascii()).unwrap();
        path
    }

    /// 単色の PNG を書く。
    pub fn write_png(&self, name: &str, size: u32, rgba: [u8; 4]) -> PathBuf {
        let path = self.dir.join(name);
        image::RgbaImage::from_pixel(size, size, image::Rgba(rgba))
            .save(&path)
            .unwrap();
        path
    }
}

/// 道を JSON の文字列に（区切りは `/`）。
pub fn slash(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Unity のマテリアルの鍵（guid は 32 桁の 16 進）。
pub fn material_key(n: u32) -> String {
    format!("guid:{n:032x}/fileid:2100000")
}

/// 試しの腕の頼み: 腕のメッシュ（Skin・Cloth の 2 つのマテリアル。BlendShape Thick）と帽子（Skin）。Lower の骨を回す。Skin は lilToon
/// （lilToon の版の値を持つ）で元の絵 `texture` を持ち、Cloth は名前が lilToon に似ているだけのシェーダーで絵が無い。
pub fn arm_request(id: &str, key: &str, fbx: &Path, texture: &Path, export_dir: &Path) -> Value {
    json!({
        "format": 1,
        "kind": "open",
        "id": id,
        "bridge": { "version": "0.5.0", "unity": "2022.3.22f1" },
        "project": { "root": "/work/Project", "name": "Project" },
        "target": { "key": key, "name": "Arm", "export_dir": slash(export_dir) },
        "models": [
            { "id": 0, "fbx": slash(fbx), "guid": "0123456789abcdef0123456789abcdef",
              "import": { "global_scale": 1.0, "use_file_scale": true, "bake_axis_conversion": false, "import_blend_shapes": true } }
        ],
        "renderers": [
            { "path": "ArmMesh", "model": 0, "node": "ArmMesh", "enabled": true, "skinned": true,
              "blend_shapes": { "Thick": 50.0 }, "materials": [0, 1] },
            { "path": "Armature/Upper/Lower/Hat", "model": 0, "node": "Armature/Upper/Lower/Hat", "enabled": true,
              "skinned": false, "materials": [0] }
        ],
        "bones": [
            { "model": 0, "node": "Armature/Upper/Lower",
              "local": { "t": [-1.0, 0.0, 0.0], "r": [0.0, 0.0, 0.38268343, 0.9238795], "s": [1.0, 1.0, 1.0] } }
        ],
        "materials": [
            { "key": material_key(1), "name": "Skin",
              "shader": { "name": "Hidden/lilToonCutout", "guid": "fedcba9876543210fedcba9876543210", "version": "2.3.4",
                          "keywords": [], "render_queue": 2450 },
              "values": { "floats": { "_Cutoff": 0.25 }, "ints": { "_lilToonVersion": 45 }, "colors": { "_Color": [1.0, 1.0, 1.0, 1.0] } },
              "textures": [ { "property": "_MainTex", "path": slash(texture), "guid": "00000000000000000000000000000001",
                              "srgb": true, "normal_map": false } ] },
            { "key": material_key(2), "name": "Cloth", "shader": { "name": "Custom/lilToonLike" } }
        ],
        "refused": [ { "path": "Accessory", "reason": "mesh_not_from_fbx" } ]
    })
}
