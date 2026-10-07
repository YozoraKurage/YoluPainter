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

/// アニメの曲線の補間（キーごとに同じもの）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    Constant,
    Linear,
    /// 3 次（接線は自動）。
    Cubic,
}

impl Interpolation {
    /// KeyAttrFlags（ufbx の読み: 定数 0x2・線形 0x4・3 次 0x8、自動の接線 0x100）。
    fn flags(self) -> u32 {
        match self {
            Interpolation::Constant => 0x2,
            Interpolation::Linear => 0x4,
            Interpolation::Cubic => 0x8 | 0x100,
        }
    }
}

/// ノードの変換のどれか（`Lcl Translation`・`Lcl Rotation`・`Lcl Scaling`）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lcl {
    Translation,
    /// オイラー角（度。XYZ の順）。
    Rotation,
    Scaling,
}

impl Lcl {
    fn property(self) -> &'static str {
        match self {
            Lcl::Translation => "Lcl Translation",
            Lcl::Rotation => "Lcl Rotation",
            Lcl::Scaling => "Lcl Scaling",
        }
    }
    fn short(self) -> &'static str {
        match self {
            Lcl::Translation => "T",
            Lcl::Rotation => "R",
            Lcl::Scaling => "S",
        }
    }
}

/// ノードの変換 1 つの曲線（x・y・z の 3 本を同じ時刻で）。
#[derive(Clone, Debug)]
pub struct NodeCurve {
    pub node: usize,
    pub lcl: Lcl,
    /// （秒、値）。
    pub keys: Vec<(f64, [f64; 3])>,
    pub interpolation: Interpolation,
}

/// BlendShape のチャンネル 1 つの DeformPercent（0〜100）の曲線。
#[derive(Clone, Debug)]
pub struct ShapeCurve {
    pub mesh: usize,
    pub channel: usize,
    /// （秒、値）。
    pub keys: Vec<(f64, f64)>,
    pub interpolation: Interpolation,
}

/// テイク（AnimationStack 1 つ・AnimationLayer 1 つ）。
#[derive(Clone, Debug, Default)]
pub struct Take {
    pub name: String,
    /// 始まりと終わり（秒。LocalStart・LocalStop）。
    pub start: f64,
    pub stop: f64,
    pub nodes: Vec<NodeCurve>,
    pub shapes: Vec<ShapeCurve>,
}

/// FBX の時刻の目盛り（1 秒の KTime）。
pub const KTIME_SECOND: f64 = 46_186_158_000.0;

fn ktime(seconds: f64) -> i64 {
    (seconds * KTIME_SECOND).round() as i64
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
    /// テイク。
    pub takes: Vec<Take>,
    /// フレームの速さ（TimeMode を任意 + CustomFrameRate で書く。None なら書かない）。
    pub frame_rate: Option<f64>,
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

    /// `Definitions` の節（種類ごとの数）。ufbx は無くても読むが、Unity（Autodesk の FBX SDK）は無いと中身を取り込まない
    /// （Unity 2022.3.22f1 で、節の無い ASCII の FBX は子の無い GameObject 1 つになった）。
    fn definitions(&self) -> String {
        let limbs = self
            .nodes
            .iter()
            .enumerate()
            .filter(|(i, n)| n.limb && !self.meshes.iter().any(|m| m.node == *i))
            .count();
        let deformers: usize = self
            .meshes
            .iter()
            .map(|m| {
                usize::from(!m.clusters.is_empty())
                    + m.clusters.len()
                    + usize::from(!m.channels.is_empty())
                    + m.channels.len()
            })
            .sum();
        let shapes: usize = self
            .meshes
            .iter()
            .flat_map(|m| &m.channels)
            .map(|c| c.shapes.len())
            .sum();
        let curve_nodes: usize = self
            .takes
            .iter()
            .map(|t| t.nodes.len() + t.shapes.len())
            .sum();
        let curves: usize = self
            .takes
            .iter()
            .map(|t| t.nodes.len() * 3 + t.shapes.len())
            .sum();
        let counts = [
            ("GlobalSettings", 1),
            ("Model", self.nodes.len()),
            ("NodeAttribute", limbs),
            ("Geometry", self.meshes.len() + shapes),
            ("Material", self.materials.len()),
            ("Deformer", deformers),
            ("AnimationStack", self.takes.len()),
            ("AnimationLayer", self.takes.len()),
            ("AnimationCurveNode", curve_nodes),
            ("AnimationCurve", curves),
        ];
        let total: usize = counts.iter().map(|(_, n)| n).sum();
        let mut o = format!("Definitions:  {{\n\tVersion: 100\n\tCount: {total}\n");
        for (kind, n) in counts.into_iter().filter(|(_, n)| *n > 0) {
            let _ = writeln!(o, "\tObjectType: \"{kind}\" {{\n\t\tCount: {n}\n\t}}");
        }
        o.push_str("}\n");
        o
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
        if let Some(fps) = self.frame_rate {
            o.push_str("\t\tP: \"TimeMode\", \"enum\", \"\", \"\",14\n");
            let _ = writeln!(
                o,
                "\t\tP: \"CustomFrameRate\", \"double\", \"Number\", \"\",{fps}"
            );
        }
        o.push_str("\t}\n}\n");
        o.push_str(&self.definitions());
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
        self.write_takes(&mut o, &mut c);
        o.push_str("}\nConnections:  {\n");
        o.push_str(&c);
        o.push_str("}\n");
        o
    }

    /// テイク（AnimationStack → AnimationLayer → AnimationCurveNode → AnimationCurve。ノードへは "OP" の Lcl の名前、
    /// BlendShape のチャンネルへは DeformPercent でつなぐ）。
    fn write_takes(&self, o: &mut String, c: &mut String) {
        let mut next = 9_000_000_000i64;
        let mut id = || {
            next += 1;
            next
        };
        let curve = |o: &mut String,
                     c: &mut String,
                     id: i64,
                     owner: i64,
                     channel: &str,
                     keys: &[(f64, f64)],
                     interpolation: Interpolation| {
            let default = keys.first().map_or(0.0, |k| k.1);
            let _ = writeln!(
                o,
                "\tAnimationCurve: {id}, \"AnimCurve::\", \"\" {{\n\t\tDefault: {default}\n\t\tKeyVer: 4009"
            );
            let times: Vec<i64> = keys.iter().map(|k| ktime(k.0)).collect();
            let values: Vec<f64> = keys.iter().map(|k| k.1).collect();
            array(o, "\t\t", "KeyTime", &times);
            array(o, "\t\t", "KeyValueFloat", &values);
            array(o, "\t\t", "KeyAttrFlags", &[interpolation.flags()]);
            array(o, "\t\t", "KeyAttrDataFloat", &[0, 0, 0, 0]);
            array(o, "\t\t", "KeyAttrRefCount", &[keys.len()]);
            o.push_str("\t}\n");
            let _ = writeln!(c, "\tC: \"OP\",{id},{owner}, \"{channel}\"");
        };
        for take in &self.takes {
            let stack = id();
            let layer = id();
            let (start, stop) = (ktime(take.start), ktime(take.stop));
            let _ = writeln!(
                o,
                "\tAnimationStack: {stack}, \"AnimStack::{}\", \"\" {{\n\t\tProperties70:  {{",
                take.name
            );
            for (name, v) in [
                ("LocalStart", start),
                ("LocalStop", stop),
                ("ReferenceStart", start),
                ("ReferenceStop", stop),
            ] {
                let _ = writeln!(o, "\t\t\tP: \"{name}\", \"KTime\", \"Time\", \"\",{v}");
            }
            o.push_str("\t\t}\n\t}\n");
            let _ = writeln!(
                o,
                "\tAnimationLayer: {layer}, \"AnimLayer::BaseLayer\", \"\" {{\n\t}}"
            );
            let _ = writeln!(c, "\tC: \"OO\",{layer},{stack}");
            for nc in &take.nodes {
                let node = id();
                let first = nc.keys.first().map_or([0.0; 3], |k| k.1);
                let _ = writeln!(
                    o,
                    "\tAnimationCurveNode: {node}, \"AnimCurveNode::{}\", \"\" {{\n\t\tProperties70:  {{",
                    nc.lcl.short()
                );
                for (axis, v) in ["X", "Y", "Z"].iter().zip(first) {
                    let _ = writeln!(o, "\t\t\tP: \"d|{axis}\", \"Number\", \"\", \"A\",{v}");
                }
                o.push_str("\t\t}\n\t}\n");
                let _ = writeln!(c, "\tC: \"OO\",{node},{layer}");
                let _ = writeln!(
                    c,
                    "\tC: \"OP\",{node},{}, \"{}\"",
                    100_000 + nc.node as i64,
                    nc.lcl.property()
                );
                for (k, axis) in ["X", "Y", "Z"].iter().enumerate() {
                    let keys: Vec<(f64, f64)> = nc.keys.iter().map(|(t, v)| (*t, v[k])).collect();
                    curve(
                        o,
                        c,
                        id(),
                        node,
                        &format!("d|{axis}"),
                        &keys,
                        nc.interpolation,
                    );
                }
            }
            for sc in &take.shapes {
                let node = id();
                let first = sc.keys.first().map_or(0.0, |k| k.1);
                let _ = writeln!(
                    o,
                    "\tAnimationCurveNode: {node}, \"AnimCurveNode::DeformPercent\", \"\" {{\n\t\tProperties70:  {{\n\t\t\tP: \"d|DeformPercent\", \"Number\", \"\", \"A\",{first}\n\t\t}}\n\t}}"
                );
                // チャンネルの番号は `to_ascii` の BlendShapeChannel と同じ式
                let channel = 400_000 + sc.mesh as i64 * 1000 + 500 + 1 + sc.channel as i64 * 20;
                let _ = writeln!(c, "\tC: \"OO\",{node},{layer}");
                let _ = writeln!(c, "\tC: \"OP\",{node},{channel}, \"DeformPercent\"");
                curve(
                    o,
                    c,
                    id(),
                    node,
                    "d|DeformPercent",
                    &sc.keys,
                    sc.interpolation,
                );
            }
        }
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

/// 試しの腕（`arm_scene`）に、テイクを 3 つ（Wave・Raise・動かす値の無い Empty）を追加したもの。30 fps。
/// Wave（0〜1 秒）: Lower の回転 z を 0 → 90 度（線形）、Upper の回転 y を 0 → 30 → 10 度（3 次）、Bend を 0 → 100（線形）。
/// Raise（0.5〜2 秒）: Upper の移動を (0, 1, 0) → (0, 1.2, 0)（定数）、Lower の大きさ x を 1 → 1.5（線形）。
pub fn arm_takes_scene() -> Scene {
    let mut s = arm_scene();
    s.frame_rate = Some(30.0);
    s.takes = vec![
        Take {
            name: "Wave".into(),
            start: 0.0,
            stop: 1.0,
            nodes: vec![
                NodeCurve {
                    node: 2,
                    lcl: Lcl::Rotation,
                    keys: vec![(0.0, [0.0; 3]), (1.0, [0.0, 0.0, 90.0])],
                    interpolation: Interpolation::Linear,
                },
                NodeCurve {
                    node: 1,
                    lcl: Lcl::Rotation,
                    keys: vec![
                        (0.0, [0.0; 3]),
                        (0.5, [0.0, 30.0, 0.0]),
                        (1.0, [0.0, 10.0, 0.0]),
                    ],
                    interpolation: Interpolation::Cubic,
                },
            ],
            shapes: vec![ShapeCurve {
                mesh: 0,
                channel: 1,
                keys: vec![(0.0, 0.0), (1.0, 100.0)],
                interpolation: Interpolation::Linear,
            }],
        },
        Take {
            name: "Raise".into(),
            start: 0.5,
            stop: 2.0,
            nodes: vec![
                NodeCurve {
                    node: 1,
                    lcl: Lcl::Translation,
                    keys: vec![(0.5, [0.0, 1.0, 0.0]), (1.5, [0.0, 1.2, 0.0])],
                    interpolation: Interpolation::Constant,
                },
                NodeCurve {
                    node: 2,
                    lcl: Lcl::Scaling,
                    keys: vec![(0.5, [1.0; 3]), (2.0, [1.5, 1.0, 1.0])],
                    interpolation: Interpolation::Linear,
                },
            ],
            shapes: Vec::new(),
        },
        Take {
            name: "Empty".into(),
            start: 0.0,
            stop: 1.0,
            ..Take::default()
        },
    ];
    s
}

/// 測りに使うアバターの形の大きさ（実のデータではなく、数だけを似せた物）。
#[derive(Clone, Debug)]
pub struct AvatarSpec {
    /// 骨の数（根の下に `chains` 本の鎖で並べる）。
    pub bones: usize,
    pub chains: usize,
    /// メッシュごとの輪の数（頂点は輪の 4 倍。どれも骨の鎖 1 本にスキンで付く）。
    pub mesh_rings: Vec<usize>,
    /// 最初のメッシュの BlendShape のチャンネルの数と、1 つの差分の頂点の数。
    pub blend_channels: usize,
    pub blend_offsets: usize,
    /// テイクの数と、1 つのテイクのフレームの数（30 fps。どの骨も毎フレーム T・R・S にキー。BlendShape は最初の 8 個に毎フレーム）。
    pub takes: usize,
    pub frames: usize,
}

/// 数だけを似せたアバターの形（骨の鎖・四角い筒のメッシュ・スキン・BlendShape・毎フレームのキーのテイク）。
pub fn avatar_scene(spec: &AvatarSpec) -> Scene {
    let mut s = Scene {
        nodes: vec![Node::new("Armature", None, [0.0; 3], false)],
        materials: vec!["Body".into(), "Cloth".into()],
        frame_rate: Some(30.0),
        ..Scene::default()
    };
    let per_chain = (spec.bones / spec.chains.max(1)).max(1);
    let mut chain_bones: Vec<Vec<usize>> = Vec::new();
    for c in 0..spec.chains {
        let mut bones = Vec::new();
        let mut parent = 0;
        for b in 0..per_chain {
            let i = s.nodes.len();
            let t = if b == 0 {
                [c as f64 * 0.1, 1.0, 0.0]
            } else {
                [0.02, 0.0, 0.0]
            };
            s.nodes
                .push(Node::new(&format!("Bone{c}_{b}"), Some(parent), t, true));
            bones.push(i);
            parent = i;
        }
        chain_bones.push(bones);
    }
    for (mi, &rings) in spec.mesh_rings.iter().enumerate() {
        let node = s.nodes.len();
        s.nodes
            .push(Node::new(&format!("Mesh{mi}"), None, [0.0; 3], false));
        let mut m = box_tube(&format!("Mesh{mi}"), node, 2.0, 1.0, 0.05, rings.max(2));
        m.materials = vec![0, 1];
        let chain = &chain_bones[mi % chain_bones.len()];
        let mut clusters: Vec<Cluster> = chain
            .iter()
            .map(|&bone| Cluster {
                bone,
                indexes: Vec::new(),
                weights: Vec::new(),
            })
            .collect();
        let n = clusters.len();
        for v in 0..m.vertices.len() {
            let ring = v / 4;
            let at = ring * n / rings.max(1);
            let (a, b) = (at.min(n - 1), (at + 1).min(n - 1));
            clusters[a].indexes.push(v as u32);
            clusters[a].weights.push(0.6);
            if b != a {
                clusters[b].indexes.push(v as u32);
                clusters[b].weights.push(0.4);
            }
        }
        m.clusters = clusters
            .into_iter()
            .filter(|c| !c.indexes.is_empty())
            .collect();
        if mi == 0 {
            let count = spec.blend_offsets.min(m.vertices.len());
            m.channels = (0..spec.blend_channels)
                .map(|k| Channel {
                    name: format!("Shape{k}"),
                    deform_percent: 0.0,
                    shapes: vec![Shape {
                        full_weight: 100.0,
                        indexes: (0..count as u32).collect(),
                        deltas: (0..count)
                            .map(|v| [0.0, 0.001 * ((v + k) % 7) as f64, 0.0])
                            .collect(),
                    }],
                })
                .collect();
        }
        s.meshes.push(m);
    }
    let fps = 30.0;
    for t in 0..spec.takes {
        let times: Vec<f64> = (0..spec.frames).map(|f| f as f64 / fps).collect();
        let mut take = Take {
            name: format!("Take{t}"),
            start: 0.0,
            stop: (spec.frames.max(1) - 1) as f64 / fps,
            ..Take::default()
        };
        for chain in &chain_bones {
            for &bone in chain {
                let base = s.nodes[bone].translation;
                for (lcl, f) in [
                    (Lcl::Translation, 0usize),
                    (Lcl::Rotation, 1),
                    (Lcl::Scaling, 2),
                ] {
                    let keys = times
                        .iter()
                        .enumerate()
                        .map(|(i, &time)| {
                            let w = (i as f64 * 0.1 + bone as f64 + t as f64).sin();
                            let v = match f {
                                0 => base,
                                1 => [w * 10.0, w * 5.0, w * 2.0],
                                _ => [1.0; 3],
                            };
                            (time, v)
                        })
                        .collect();
                    take.nodes.push(NodeCurve {
                        node: bone,
                        lcl,
                        keys,
                        interpolation: Interpolation::Cubic,
                    });
                }
            }
        }
        for k in 0..spec.blend_channels.min(8) {
            take.shapes.push(ShapeCurve {
                mesh: 0,
                channel: k,
                keys: times
                    .iter()
                    .enumerate()
                    .map(|(i, &time)| (time, 50.0 + 50.0 * (i as f64 * 0.2).sin()))
                    .collect(),
                interpolation: Interpolation::Cubic,
            });
        }
        s.takes.push(take);
    }
    s
}
