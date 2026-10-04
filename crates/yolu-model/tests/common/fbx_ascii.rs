//! 試験の FBX をコードで組む（FBX 7.4 の ASCII。手で書くのと同じ中身を、数が多くても間違えないように）。実のアバターのデータは
//! 使わない。座標はファイルの空間（右手系・Y が上・単位はメートル = UnitScaleFactor 100）。
#![allow(dead_code)]

use std::fmt::Write;

use yolu_core::glam::{DMat4, DQuat, DVec3, EulerRot};

/// ノード（Null・LimbNode・メッシュの付くノード）。
#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: [f64; 3],
    /// オイラー角（度。FBX の既定の XYZ の順）。
    pub rotation: [f64; 3],
    pub scale: [f64; 3],
    pub limb: bool,
}

impl Node {
    pub fn new(name: &str, parent: Option<usize>, translation: [f64; 3], limb: bool) -> Node {
        Node {
            name: name.into(),
            parent,
            translation,
            rotation: [0.0; 3],
            scale: [1.0; 3],
            limb,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Cluster {
    pub bone: usize,
    pub indexes: Vec<u32>,
    pub weights: Vec<f64>,
}

#[derive(Clone, Debug)]
pub struct Shape {
    pub full_weight: f64,
    pub indexes: Vec<u32>,
    pub deltas: Vec<[f64; 3]>,
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub name: String,
    pub deform_percent: f64,
    pub shapes: Vec<Shape>,
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub name: String,
    /// 付くノード。
    pub node: usize,
    pub vertices: Vec<[f64; 3]>,
    pub polygons: Vec<Vec<u32>>,
    /// ポリゴンの頂点ごと（並びは polygons の順）。
    pub uvs: Option<Vec<[f64; 2]>>,
    pub normals: Option<Vec<[f64; 3]>>,
    /// ポリゴンごとのマテリアルのスロット（空なら付けない）。
    pub polygon_materials: Vec<u32>,
    /// スロット → シーンのマテリアルの番号。
    pub materials: Vec<usize>,
    pub clusters: Vec<Cluster>,
    pub channels: Vec<Channel>,
}

#[derive(Clone, Debug, Default)]
pub struct Scene {
    pub nodes: Vec<Node>,
    pub meshes: Vec<Mesh>,
    pub materials: Vec<String>,
    /// 単位をセンチメートルにする（UnitScaleFactor 1。既定はメートル = 100）。
    pub centimeters: bool,
    /// Z が上・−Y が前の右手系（Blender の書き出しの既定）にする（既定は Y が上・+Z が前）。
    pub z_up: bool,
}

fn array<T: std::fmt::Display>(out: &mut String, indent: &str, name: &str, values: &[T]) {
    let _ = write!(out, "{indent}{name}: *{} {{\n{indent}\ta: ", values.len());
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let _ = write!(out, "{v}");
    }
    let _ = writeln!(out, "\n{indent}}}");
}

/// ノードのローカル（T × R × S、R は XYZ の順 = Rz × Ry × Rx）。
pub fn local(n: &Node) -> DMat4 {
    let r = DQuat::from_euler(
        EulerRot::ZYX,
        n.rotation[2].to_radians(),
        n.rotation[1].to_radians(),
        n.rotation[0].to_radians(),
    );
    DMat4::from_scale_rotation_translation(
        DVec3::from_array(n.scale),
        r,
        DVec3::from_array(n.translation),
    )
}

impl Scene {
    /// 同じ形を別の空間で書く: 座標を f で写し、長さを k 倍する（ノードの回転は 0 のときだけ正しい）。
    pub fn transformed(&self, f: impl Fn([f64; 3]) -> [f64; 3], k: f64) -> Scene {
        let g = |p: [f64; 3]| f([p[0] * k, p[1] * k, p[2] * k]);
        let mut s = self.clone();
        for n in &mut s.nodes {
            assert_eq!(n.rotation, [0.0; 3]);
            n.translation = g(n.translation);
        }
        for m in &mut s.meshes {
            m.vertices.iter_mut().for_each(|v| *v = g(*v));
            if let Some(normals) = &mut m.normals {
                normals.iter_mut().for_each(|n| *n = f(*n));
            }
            for ch in &mut m.channels {
                for sh in &mut ch.shapes {
                    sh.deltas.iter_mut().for_each(|d| *d = g(*d));
                }
            }
        }
        s
    }

    pub fn world(&self, node: usize) -> DMat4 {
        let n = &self.nodes[node];
        match n.parent {
            Some(p) => self.world(p) * local(n),
            None => local(n),
        }
    }

    pub fn to_ascii(&self) -> String {
        let mut o = String::new();
        o.push_str("; FBX 7.4.0 project file\n");
        o.push_str("FBXHeaderExtension:  {\n\tFBXHeaderVersion: 1003\n\tFBXVersion: 7400\n\tCreator: \"yolu-model tests\"\n}\n");
        o.push_str("GlobalSettings:  {\n\tVersion: 1000\n\tProperties70:  {\n");
        let (up, front, front_sign) = if self.z_up { (2, 1, -1) } else { (1, 2, 1) };
        for (name, v) in [
            ("UpAxis", up),
            ("UpAxisSign", 1),
            ("FrontAxis", front),
            ("FrontAxisSign", front_sign),
            ("CoordAxis", 0),
            ("CoordAxisSign", 1),
        ] {
            let _ = writeln!(o, "\t\tP: \"{name}\", \"int\", \"Integer\", \"\",{v}");
        }
        let unit = if self.centimeters { 1 } else { 100 };
        let _ = writeln!(
            o,
            "\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",{unit}"
        );
        o.push_str("\t}\n}\n");
        o.push_str("Objects:  {\n");
        let mut c = String::new(); // Connections
        let model_id = |i: usize| 100_000 + i as i64;
        for (i, n) in self.nodes.iter().enumerate() {
            let has_mesh = self.meshes.iter().any(|m| m.node == i);
            let kind = if has_mesh {
                "Mesh"
            } else if n.limb {
                "LimbNode"
            } else {
                "Null"
            };
            if n.limb && !has_mesh {
                let _ = writeln!(
                    o,
                    "\tNodeAttribute: {}, \"NodeAttribute::{}\", \"LimbNode\" {{\n\t\tTypeFlags: \"Skeleton\"\n\t}}",
                    200_000 + i,
                    n.name
                );
                let _ = writeln!(c, "\tC: \"OO\",{},{}", 200_000 + i, model_id(i));
            }
            let _ = writeln!(
                o,
                "\tModel: {}, \"Model::{}\", \"{kind}\" {{\n\t\tVersion: 232\n\t\tProperties70:  {{",
                model_id(i),
                n.name
            );
            let t = n.translation;
            let r = n.rotation;
            let s = n.scale;
            let _ = writeln!(
                o,
                "\t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",{},{},{}",
                t[0], t[1], t[2]
            );
            let _ = writeln!(
                o,
                "\t\t\tP: \"Lcl Rotation\", \"Lcl Rotation\", \"\", \"A\",{},{},{}",
                r[0], r[1], r[2]
            );
            let _ = writeln!(
                o,
                "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",{},{},{}",
                s[0], s[1], s[2]
            );
            o.push_str("\t\t}\n\t\tShading: T\n\t\tCulling: \"CullingOff\"\n\t}\n");
            let parent = n.parent.map(model_id).unwrap_or(0);
            let _ = writeln!(c, "\tC: \"OO\",{},{parent}", model_id(i));
        }
        for (i, name) in self.materials.iter().enumerate() {
            let _ = writeln!(
                o,
                "\tMaterial: {}, \"Material::{name}\", \"\" {{\n\t\tVersion: 102\n\t\tShadingModel: \"phong\"\n\t\tMultiLayer: 0\n\t}}",
                300_000 + i
            );
        }
        for (mi, m) in self.meshes.iter().enumerate() {
            let gid = 400_000 + mi as i64 * 1000;
            let _ = writeln!(
                o,
                "\tGeometry: {gid}, \"Geometry::{}\", \"Mesh\" {{",
                m.name
            );
            let flat: Vec<f64> = m.vertices.iter().flatten().copied().collect();
            array(&mut o, "\t\t", "Vertices", &flat);
            let mut pvi = Vec::new();
            for p in &m.polygons {
                for (k, &v) in p.iter().enumerate() {
                    pvi.push(if k + 1 == p.len() {
                        -(v as i64) - 1
                    } else {
                        v as i64
                    });
                }
            }
            array(&mut o, "\t\t", "PolygonVertexIndex", &pvi);
            o.push_str("\t\tGeometryVersion: 124\n");
            let mut layers = Vec::new();
            if let Some(normals) = &m.normals {
                o.push_str("\t\tLayerElementNormal: 0 {\n\t\t\tVersion: 102\n\t\t\tName: \"\"\n\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\t\t\tReferenceInformationType: \"Direct\"\n");
                let flat: Vec<f64> = normals.iter().flatten().copied().collect();
                array(&mut o, "\t\t\t", "Normals", &flat);
                o.push_str("\t\t}\n");
                layers.push("LayerElementNormal");
            }
            if let Some(uvs) = &m.uvs {
                o.push_str("\t\tLayerElementUV: 0 {\n\t\t\tVersion: 101\n\t\t\tName: \"UVMap\"\n\t\t\tMappingInformationType: \"ByPolygonVertex\"\n\t\t\tReferenceInformationType: \"IndexToDirect\"\n");
                let flat: Vec<f64> = uvs.iter().flatten().copied().collect();
                array(&mut o, "\t\t\t", "UV", &flat);
                let index: Vec<u32> = (0..uvs.len() as u32).collect();
                array(&mut o, "\t\t\t", "UVIndex", &index);
                o.push_str("\t\t}\n");
                layers.push("LayerElementUV");
            }
            if !m.polygon_materials.is_empty() {
                o.push_str("\t\tLayerElementMaterial: 0 {\n\t\t\tVersion: 101\n\t\t\tName: \"\"\n\t\t\tMappingInformationType: \"ByPolygon\"\n\t\t\tReferenceInformationType: \"IndexToDirect\"\n");
                array(&mut o, "\t\t\t", "Materials", &m.polygon_materials);
                o.push_str("\t\t}\n");
                layers.push("LayerElementMaterial");
            }
            o.push_str("\t\tLayer: 0 {\n\t\t\tVersion: 100\n");
            for l in layers {
                let _ = writeln!(o, "\t\t\tLayerElement:  {{\n\t\t\t\tType: \"{l}\"\n\t\t\t\tTypedIndex: 0\n\t\t\t}}");
            }
            o.push_str("\t\t}\n\t}\n");
            let _ = writeln!(c, "\tC: \"OO\",{gid},{}", model_id(m.node));
            for &mat in &m.materials {
                let _ = writeln!(c, "\tC: \"OO\",{},{}", 300_000 + mat, model_id(m.node));
            }
            if !m.clusters.is_empty() {
                let skin = gid + 1;
                let _ = writeln!(o, "\tDeformer: {skin}, \"Deformer::\", \"Skin\" {{\n\t\tVersion: 101\n\t\tLink_DeformAcuracy: 50\n\t}}");
                let _ = writeln!(c, "\tC: \"OO\",{skin},{gid}");
                let mesh_world = self.world(m.node);
                for (k, cl) in m.clusters.iter().enumerate() {
                    let id = gid + 10 + k as i64;
                    let link = self.world(cl.bone);
                    let transform = link.inverse() * mesh_world;
                    let _ = writeln!(o, "\tDeformer: {id}, \"SubDeformer::\", \"Cluster\" {{\n\t\tVersion: 100\n\t\tUserData: \"\", \"\"");
                    array(&mut o, "\t\t", "Indexes", &cl.indexes);
                    array(&mut o, "\t\t", "Weights", &cl.weights);
                    array(&mut o, "\t\t", "Transform", &transform.to_cols_array());
                    array(&mut o, "\t\t", "TransformLink", &link.to_cols_array());
                    o.push_str("\t}\n");
                    let _ = writeln!(c, "\tC: \"OO\",{id},{skin}");
                    let _ = writeln!(c, "\tC: \"OO\",{},{id}", model_id(cl.bone));
                }
            }
            if !m.channels.is_empty() {
                let bs = gid + 500;
                let _ = writeln!(
                    o,
                    "\tDeformer: {bs}, \"Deformer::{}\", \"BlendShape\" {{\n\t\tVersion: 100\n\t}}",
                    m.name
                );
                let _ = writeln!(c, "\tC: \"OO\",{bs},{gid}");
                for (k, ch) in m.channels.iter().enumerate() {
                    let id = bs + 1 + k as i64 * 20;
                    let _ = writeln!(o, "\tDeformer: {id}, \"SubDeformer::{}\", \"BlendShapeChannel\" {{\n\t\tVersion: 100\n\t\tDeformPercent: {}", ch.name, ch.deform_percent);
                    let fw: Vec<f64> = ch.shapes.iter().map(|s| s.full_weight).collect();
                    array(&mut o, "\t\t", "FullWeights", &fw);
                    o.push_str("\t}\n");
                    let _ = writeln!(c, "\tC: \"OO\",{id},{bs}");
                    for (j, s) in ch.shapes.iter().enumerate() {
                        let sid = id + 1 + j as i64;
                        let _ = writeln!(o, "\tGeometry: {sid}, \"Geometry::{}_{j}\", \"Shape\" {{\n\t\tVersion: 100", ch.name);
                        array(&mut o, "\t\t", "Indexes", &s.indexes);
                        let flat: Vec<f64> = s.deltas.iter().flatten().copied().collect();
                        array(&mut o, "\t\t", "Vertices", &flat);
                        o.push_str("\t}\n");
                        let _ = writeln!(c, "\tC: \"OO\",{sid},{id}");
                    }
                }
            }
        }
        o.push_str("}\nConnections:  {\n");
        o.push_str(&c);
        o.push_str("}\n");
        o
    }
}

/// 四角い断面の筒（x = 0..length、y = y0、断面の半分の幅 h）。輪は x の等間隔で rings 個、面ごとに平らな法線と UV
/// （u = x / length、v = 面の番号 / 4 の帯）。返すのは メッシュと、輪ごとの頂点の番号（輪 r の頂点は 4r..4r+4）。
pub fn box_tube(name: &str, node: usize, length: f64, y0: f64, h: f64, rings: usize) -> Mesh {
    let mut vertices = Vec::new();
    for r in 0..rings {
        let x = length * r as f64 / (rings - 1) as f64;
        for (dy, dz) in [(h, h), (-h, h), (-h, -h), (h, -h)] {
            vertices.push([x, y0 + dy, dz]);
        }
    }
    let mut polygons = Vec::new();
    let mut uvs = Vec::new();
    let mut normals = Vec::new();
    let mut polygon_materials = Vec::new();
    // 面 k は頂点 k と k+1（輪の中）を結ぶ側面。外向きの法線
    let face_normals = [
        [0.0, 0.0, 1.0],
        [0.0, -1.0, 0.0],
        [0.0, 0.0, -1.0],
        [0.0, 1.0, 0.0],
    ];
    for r in 0..rings - 1 {
        for k in 0..4u32 {
            let a = (4 * r) as u32 + k;
            let b = (4 * r) as u32 + (k + 1) % 4;
            let (c, d) = (a + 4, b + 4);
            // 右手系で外から見て反時計回り: a → b → d → c
            polygons.push(vec![a, b, d, c]);
            let u0 = r as f64 / (rings - 1) as f64;
            let u1 = (r + 1) as f64 / (rings - 1) as f64;
            let (v0, v1) = (k as f64 / 4.0, (k + 1) as f64 / 4.0);
            uvs.extend_from_slice(&[[u0, v0], [u0, v1], [u1, v1], [u1, v0]]);
            for _ in 0..4 {
                normals.push(face_normals[k as usize]);
            }
            polygon_materials.push(if k < 2 { 0 } else { 1 });
        }
    }
    Mesh {
        name: name.into(),
        node,
        vertices,
        polygons,
        uvs: Some(uvs),
        normals: Some(normals),
        polygon_materials,
        materials: Vec::new(),
        clusters: Vec::new(),
        channels: Vec::new(),
    }
}

/// 試しの腕: 根 → Armature（Null）→ Upper（y = 1）→ Lower（x = 1）。メッシュ ArmMesh は根の子で、x = 0..2 の四角い筒
/// （輪 5 つ）を Upper・Lower に塗り分け（x < 1 は Upper、x = 1 は半々、x > 1 は Lower）、マテリアルは Skin（上と手前の面）と
/// Cloth。BlendShape は Thick（x = 1.5 の輪を外へ 0.1。DeformPercent 25）と Bend（中間のフレーム 50・100）。Lower の子の Hat は
/// スキンの無い三角形 1 つ（Skin）。
pub fn arm_scene() -> Scene {
    let mut s = Scene {
        nodes: vec![
            Node::new("Armature", None, [0.0, 0.0, 0.0], false),
            Node::new("Upper", Some(0), [0.0, 1.0, 0.0], true),
            Node::new("Lower", Some(1), [1.0, 0.0, 0.0], true),
            Node::new("ArmMesh", None, [0.0, 0.0, 0.0], false),
            Node::new("Hat", Some(2), [0.5, 0.2, 0.0], false),
        ],
        materials: vec!["Skin".into(), "Cloth".into()],
        ..Scene::default()
    };
    let mut arm = box_tube("ArmMesh", 3, 2.0, 1.0, 0.1, 5);
    arm.materials = vec![0, 1];
    let mut upper = Cluster {
        bone: 1,
        indexes: Vec::new(),
        weights: Vec::new(),
    };
    let mut lower = Cluster {
        bone: 2,
        indexes: Vec::new(),
        weights: Vec::new(),
    };
    for (v, p) in arm.vertices.iter().enumerate() {
        let (wu, wl) = if p[0] < 0.99 {
            (1.0, 0.0)
        } else if p[0] < 1.01 {
            (0.5, 0.5)
        } else {
            (0.0, 1.0)
        };
        if wu > 0.0 {
            upper.indexes.push(v as u32);
            upper.weights.push(wu);
        }
        if wl > 0.0 {
            lower.indexes.push(v as u32);
            lower.weights.push(wl);
        }
    }
    arm.clusters = vec![upper, lower];
    let ring = |r: u32| (4 * r..4 * r + 4).collect::<Vec<u32>>();
    let outward = |v: u32, amount: f64, vertices: &[[f64; 3]]| {
        let p = vertices[v as usize];
        [0.0, (p[1] - 1.0) / 0.1 * amount, p[2] / 0.1 * amount]
    };
    arm.channels = vec![
        Channel {
            name: "Thick".into(),
            deform_percent: 25.0,
            shapes: vec![Shape {
                full_weight: 100.0,
                indexes: ring(3),
                deltas: ring(3)
                    .iter()
                    .map(|&v| outward(v, 0.1, &arm.vertices))
                    .collect(),
            }],
        },
        Channel {
            name: "Bend".into(),
            deform_percent: 0.0,
            shapes: vec![
                Shape {
                    full_weight: 50.0,
                    indexes: ring(4),
                    deltas: vec![[0.0, 0.2, 0.0]; 4],
                },
                Shape {
                    full_weight: 100.0,
                    indexes: ring(4),
                    deltas: vec![[0.0, 0.6, 0.1]; 4],
                },
            ],
        },
    ];
    s.meshes.push(arm);
    s.meshes.push(Mesh {
        name: "Hat".into(),
        node: 4,
        vertices: vec![[0.0, 0.0, 0.0], [0.1, 0.0, 0.0], [0.0, 0.1, 0.0]],
        polygons: vec![vec![0, 1, 2]],
        uvs: Some(vec![[0.9, 0.9], [1.0, 0.9], [0.9, 1.0]]),
        normals: None,
        polygon_materials: vec![0],
        materials: vec![0],
        clusters: Vec::new(),
        channels: Vec::new(),
    });
    s
}
