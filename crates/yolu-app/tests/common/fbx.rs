//! 試験用の小さな ASCII の FBX（四角い板をメッシュごとに並べ、板ごとにマテリアルを当てる）。ufbx が読める最小の形で、
//! 材料の無い板はマテリアルを持たない（読み込みが「マテリアルなし」にする）。
#![allow(dead_code)]

use std::fmt::Write;
use std::path::{Path, PathBuf};

/// メッシュ 1 つ: 名前と、板ごとのマテリアルの番号（`None` はマテリアルなし）。
pub struct MeshSpec<'a> {
    pub name: &'a str,
    pub quads: Vec<Option<usize>>,
}

pub fn mesh<'a>(name: &'a str, quads: &[Option<usize>]) -> MeshSpec<'a> {
    MeshSpec {
        name,
        quads: quads.to_vec(),
    }
}

/// マテリアルの名前の並びと、メッシュの並びから FBX（ASCII）を作る。
pub fn ascii_fbx(materials: &[&str], meshes: &[MeshSpec<'_>]) -> String {
    let mut o = String::new();
    o.push_str("; FBX 7.4.0 project file\nFBXHeaderExtension:  {\n\tFBXHeaderVersion: 1003\n\tFBXVersion: 7400\n}\n");
    o.push_str("GlobalSettings:  {\n\tVersion: 1000\n\tProperties70:  {\n");
    for (name, v) in [
        ("UpAxis", 1),
        ("UpAxisSign", 1),
        ("FrontAxis", 2),
        ("FrontAxisSign", 1),
        ("CoordAxis", 0),
        ("CoordAxisSign", 1),
    ] {
        let _ = writeln!(o, "\t\tP: \"{name}\", \"int\", \"Integer\", \"\",{v}");
    }
    o.push_str("\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",100\n\t}\n}\n");
    o.push_str("Objects:  {\n");
    let mut c = String::new();
    for (i, name) in materials.iter().enumerate() {
        let _ = writeln!(
            o,
            "\tMaterial: {}, \"Material::{name}\", \"\" {{\n\t\tVersion: 102\n\t\tShadingModel: \"phong\"\n\t\tMultiLayer: 0\n\t}}",
            300_000 + i
        );
    }
    for (mi, m) in meshes.iter().enumerate() {
        let model = 100_000 + mi as i64;
        let geometry = 400_000 + mi as i64;
        let _ = writeln!(
            o,
            "\tModel: {model}, \"Model::{}\", \"Mesh\" {{\n\t\tVersion: 232\n\t\tProperties70:  {{\n\t\t}}\n\t\tShading: T\n\t\tCulling: \"CullingOff\"\n\t}}",
            m.name
        );
        let n = m.quads.len().max(1);
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut uvs = Vec::new();
        for q in 0..n {
            let x = q as f64 + mi as f64 * 100.0;
            for (dx, dy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                vertices.push(format!("{},{},0", x + dx, dy));
            }
            let base = q * 4;
            indices.push(format!(
                "{},{},{},{}",
                base,
                base + 1,
                base + 2,
                -((base + 3) as i64) - 1
            ));
            let (u0, u1) = (q as f64 / n as f64, (q + 1) as f64 / n as f64);
            for (u, v) in [(u0, 0.0), (u1, 0.0), (u1, 1.0), (u0, 1.0)] {
                uvs.push(format!("{u},{v}"));
            }
        }
        let _ = writeln!(
            o,
            "\tGeometry: {geometry}, \"Geometry::{}\", \"Mesh\" {{",
            m.name
        );
        let _ = writeln!(
            o,
            "\t\tVertices: *{} {{\n\t\t\ta: {}\n\t\t}}",
            vertices.len() * 3,
            vertices.join(",")
        );
        let _ = writeln!(
            o,
            "\t\tPolygonVertexIndex: *{} {{\n\t\t\ta: {}\n\t\t}}",
            indices.len() * 4,
            indices.join(",")
        );
        o.push_str("\t\tGeometryVersion: 124\n");
        let _ = writeln!(
            o,
            "\t\tLayerElementUV: 0 {{\n\t\t\tVersion: 101\n\t\t\tName: \"UVMap\"\n\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\t\t\tReferenceInformationType: \"Direct\"\n\t\t\tUV: *{} {{\n\t\t\t\ta: {}\n\t\t\t}}\n\t\t}}",
            uvs.len() * 2,
            uvs.join(",")
        );
        // このメッシュが使うマテリアル（出てきた順）。スロットの番号はこの並び
        let mut used: Vec<usize> = Vec::new();
        for quad in m.quads.iter().flatten() {
            if !used.contains(quad) {
                used.push(*quad);
            }
        }
        let mut layers = vec!["LayerElementUV"];
        if !used.is_empty() && m.quads.iter().all(Option::is_some) {
            let slots: Vec<String> = m
                .quads
                .iter()
                .map(|q| {
                    used.iter()
                        .position(|u| Some(*u) == *q)
                        .unwrap()
                        .to_string()
                })
                .collect();
            let _ = writeln!(
                o,
                "\t\tLayerElementMaterial: 0 {{\n\t\t\tVersion: 101\n\t\t\tName: \"\"\n\t\t\tMappingInformationType: \"ByPolygon\"\n\t\t\tReferenceInformationType: \"IndexToDirect\"\n\t\t\tMaterials: *{} {{\n\t\t\t\ta: {}\n\t\t\t}}\n\t\t}}",
                slots.len(),
                slots.join(",")
            );
            layers.push("LayerElementMaterial");
        }
        o.push_str("\t\tLayer: 0 {\n\t\t\tVersion: 100\n");
        for l in layers {
            let _ = writeln!(
                o,
                "\t\t\tLayerElement:  {{\n\t\t\t\tType: \"{l}\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}}"
            );
        }
        o.push_str("\t\t}\n\t}\n");
        let _ = writeln!(c, "\tC: \"OO\",{model},0");
        let _ = writeln!(c, "\tC: \"OO\",{geometry},{model}");
        if m.quads.iter().all(Option::is_some) {
            for u in &used {
                let _ = writeln!(c, "\tC: \"OO\",{},{model}", 300_000 + u);
            }
        }
    }
    o.push_str("}\nConnections:  {\n");
    o.push_str(&c);
    o.push_str("}\n");
    o
}

/// FBX を一時のフォルダに書く（プロセスごとのフォルダ。名前は呼ぶ側が決める）。
pub fn write_fbx(dir: &Path, file: &str, text: &str) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(file);
    std::fs::write(&path, text).unwrap();
    path
}

/// 試験ごとの一時のフォルダ（試験の名前とプロセスで一意）。
pub fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yolu-newproject-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    crate::common::tmp::clean_up_after_test(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
