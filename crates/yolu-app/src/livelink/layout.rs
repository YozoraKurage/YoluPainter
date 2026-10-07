//! 頼みの道（Unity の根からの名前の道）を、まとめた `Rig` の骨とメッシュへ引く。ポーズ・BlendShape の重みを頼みから作り、今のポーズを
//! 頼みの形（骨の道と値・レンダラーごとの BlendShape の重み）へ戻す（.ylp に残す形）。
//!
//! - Unity の根: FBX の根のノード（yolu-model の根の骨）。ただし Unity は、取り込みの `preserveHierarchy` が切れていて根の子が 1 つだけの
//!   FBX では、その子をプレハブの根にする（その子の名前は道から消え、その子の変換はプレハブの根に移る）。そのときは、その子を Unity の根に
//!   する。道の空の文字列は Unity の根。
//! - 道は Unity の根から子の名前を 1 つずつ下る。同じ親の下に同じ名前の子が 2 つ以上あれば決まらない（`ambiguous_bone`）。Unity は取り込みで
//!   同じ名前の兄弟を `Twin`・`Twin 1` のように付け直すので、FBX に無い `<名前> <数>` の節で、FBX にその名前の兄弟が 2 つ以上あるときも
//!   決まらない（どれに付け直したかは FBX からは分からない）。
//! - 骨の値は、FBX の親の骨に対するローカル（Unity の左手系・メートル。yolu-model の休みのローカルと同じ決まり）をそのまま入れる。
//!   頼みに無い骨は休みのまま。

use std::ops::Range;

use yolu_core::glam::{Quat, Vec3};
use yolu_core::skin::{BoneTransform, Pose, Rig};
use yolu_protocol::files::{BoneValue, Local, Problem, Reason, Request};

/// FBX 1 つ（`models[]` の 1 つ）が、まとめた Rig のどこにあるか。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartLayout {
    /// `models[].id`。
    pub model: u32,
    pub bones: Range<usize>,
    /// Unity の根に当たる骨（まとめた Rig の番号）。
    pub unity_root: usize,
}

/// まとめた Rig と頼みの対応。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Layout {
    pub parts: Vec<PartLayout>,
    /// `renderers[]` ごとの、まとめた Rig のメッシュの番号（入れなかったレンダラーは None）。
    pub renderers: Vec<Option<usize>>,
}

/// 道が骨に決まらない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMiss {
    NotFound,
    Ambiguous,
}

impl PathMiss {
    pub fn reason(self) -> Reason {
        match self {
            PathMiss::NotFound => Reason::BoneNotFound,
            PathMiss::Ambiguous => Reason::AmbiguousBone,
        }
    }
}

/// Unity の根に当たる骨（`root` は FBX の根の骨）。畳む設定で根の子が 1 つだけなら、その子。
pub fn unity_root(rig: &Rig, root: usize, preserve_hierarchy: bool) -> usize {
    match rig.children(root) {
        [only] if !preserve_hierarchy => *only as usize,
        _ => root,
    }
}

/// Unity の根 `from` からの名前の道（`/` 区切り。空は `from` そのもの）を骨にする。
pub fn resolve(rig: &Rig, from: usize, path: &str) -> Result<usize, PathMiss> {
    let mut at = from;
    if path.is_empty() {
        return Ok(at);
    }
    for name in path.split('/') {
        let named = |name: &str| {
            rig.children(at)
                .iter()
                .map(|&c| c as usize)
                .filter(|&c| rig.bones()[c].name == name)
                .collect::<Vec<_>>()
        };
        match named(name)[..] {
            [one] => at = one,
            [] => {
                // Unity が付け直した名前（`Twin 1`）: FBX にその元の名前の兄弟が 2 つ以上あれば、どれか決まらない
                let renamed = renamed_base(name).is_some_and(|base| named(base).len() > 1);
                return Err(if renamed {
                    PathMiss::Ambiguous
                } else {
                    PathMiss::NotFound
                });
            }
            _ => return Err(PathMiss::Ambiguous),
        }
    }
    Ok(at)
}

/// Unity が同じ名前の兄弟に付け直す形（`<名前> <数>`）なら、元の名前。
fn renamed_base(name: &str) -> Option<&str> {
    let (base, n) = name.rsplit_once(' ')?;
    (!base.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then_some(base)
}

/// 骨 `bone` の、Unity の根 `from` からの名前の道（`from` の下でなければ None）。
pub fn path_of(rig: &Rig, from: usize, bone: usize) -> Option<String> {
    let mut names = Vec::new();
    let mut at = bone;
    while at != from {
        names.push(rig.bones()[at].name.as_str());
        at = rig.bones()[at].parent? as usize;
    }
    names.reverse();
    Some(names.join("/"))
}

/// 骨の値を変換にする（回転は長さ 1 に直す。0 の回転は回さない）。
pub fn transform_of(local: &Local) -> BoneTransform {
    let r = Quat::from_array(local.r);
    BoneTransform {
        translation: Vec3::from_array(local.t),
        rotation: if r.length_squared() > 1e-12 {
            r.normalize()
        } else {
            Quat::IDENTITY
        },
        scale: Vec3::from_array(local.s),
    }
}

fn local_of(t: &BoneTransform) -> Local {
    Local {
        t: t.translation.to_array(),
        r: t.rotation.to_array(),
        s: t.scale.to_array(),
    }
}

impl Layout {
    fn part(&self, model: u32) -> Option<&PartLayout> {
        self.parts.iter().find(|p| p.model == model)
    }

    /// 骨の値の道を骨にする。
    pub fn bone(&self, rig: &Rig, value: &BoneValue) -> Result<usize, PathMiss> {
        let part = self.part(value.model).ok_or(PathMiss::NotFound)?;
        resolve(rig, part.unity_root, &value.node)
    }

    /// 頼みの骨の値と BlendShape の重みを当てたポーズ（休みのポーズから）。合わなかった骨は理由を返す（同じ道は 1 つにまとめる）。
    /// 知らない BlendShape の名前は飛ばす（Unity の取り込みと FBX のチャンネルの名前は同じ。合わないのは別の FBX）。
    pub fn pose(&self, rig: &Rig, request: &Request) -> (Pose, Vec<Problem>) {
        let mut pose = rig.rest_pose();
        let mut problems: Vec<Problem> = Vec::new();
        for value in &request.bones {
            match self.bone(rig, value) {
                Ok(bone) => pose.locals[bone] = transform_of(&value.local),
                Err(miss) => {
                    let p = Problem::new(value.node.clone(), miss.reason());
                    if !problems.contains(&p) {
                        problems.push(p);
                    }
                }
            }
        }
        for (renderer, mesh) in request.renderers.iter().zip(&self.renderers) {
            let Some(mesh) = *mesh else { continue };
            let shapes = &rig.meshes()[mesh].blend_shapes;
            for (name, weight) in &renderer.blend_shapes {
                if let Some(k) = shapes.iter().position(|s| &s.name == name) {
                    pose.blend_weights[mesh][k] = *weight;
                }
            }
        }
        (pose, problems)
    }

    /// 今のポーズを頼みの形へ戻す（休みと違う骨の値、レンダラーごとの全部の BlendShape の重み）。書けなかった骨の名前を返す: Unity の根の外の
    /// 骨（畳んだ FBX の根の骨）と、道で引き直せない骨（同じ名前の兄弟）は、道で指せないので書かない。
    pub fn write_pose(&self, rig: &Rig, pose: &Pose, request: &mut Request) -> Vec<String> {
        let rest = rig.rest_pose();
        let mut unsaved = Vec::new();
        request.bones.clear();
        for part in &self.parts {
            for bone in part.bones.clone() {
                if pose.locals[bone] == rest.locals[bone] {
                    continue;
                }
                let path = path_of(rig, part.unity_root, bone)
                    .filter(|path| resolve(rig, part.unity_root, path) == Ok(bone));
                let Some(path) = path else {
                    unsaved.push(rig.bones()[bone].name.clone());
                    continue;
                };
                request.bones.push(BoneValue {
                    model: part.model,
                    node: path,
                    local: local_of(&pose.locals[bone]),
                });
            }
        }
        for (renderer, mesh) in request.renderers.iter_mut().zip(&self.renderers) {
            let Some(mesh) = *mesh else { continue };
            renderer.blend_shapes = rig.meshes()[mesh]
                .blend_shapes
                .iter()
                .zip(&pose.blend_weights[mesh])
                .map(|(s, w)| (s.name.clone(), *w))
                .collect();
        }
        unsaved
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yolu_core::geometry::{ModelMesh, Submesh};
    use yolu_core::skin::{Bone, RigBudget, RigMesh, Skin};

    /// 根 → Armature → {Hips → Spine, Hips2}、根 → Body（メッシュ）。Armature の下に同じ名前の兄弟 Twin が 2 つ。
    fn rig(single_top: bool) -> Rig {
        let b = |name: &str, parent: Option<u32>| Bone {
            name: name.into(),
            parent,
            rest: BoneTransform::IDENTITY,
        };
        let mut bones = vec![
            b("file", None),
            b("Armature", Some(0)),
            b("Hips", Some(1)),
            b("Spine", Some(2)),
            b("Twin", Some(1)),
            b("Twin", Some(1)),
        ];
        let body_parent = if single_top { 1 } else { 0 };
        bones.push(b("Body", Some(body_parent)));
        let mesh = ModelMesh {
            name: "Body".into(),
            positions: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            normals: vec![],
            uvs: vec![],
            submeshes: vec![Submesh {
                material: 0,
                indices: vec![0, 1, 2],
            }],
        };
        let shape = |name: &str| yolu_core::skin::BlendShape {
            name: name.into(),
            frames: vec![yolu_core::skin::BlendFrame {
                weight: 100.0,
                vertices: vec![0],
                positions: vec![Vec3::Y],
                normals: vec![],
            }],
        };
        Rig::new(
            "file",
            bones,
            vec![RigMesh {
                mesh,
                skin: Skin::rigid(6, yolu_core::glam::Mat4::IDENTITY, 3),
                blend_shapes: vec![shape("smile"), shape("blink")],
                node: Some(6),
            }],
            vec!["m".into()],
            Vec::new(),
            &RigBudget::default(),
        )
        .unwrap()
    }

    #[test]
    fn the_unity_root_is_the_only_child_when_the_hierarchy_is_collapsed() {
        let one = rig(true);
        assert_eq!(
            unity_root(&one, 0, false),
            1,
            "根の子は Armature だけ: 畳む"
        );
        assert_eq!(
            unity_root(&one, 0, true),
            0,
            "preserveHierarchy なら畳まない"
        );
        let two = rig(false);
        assert_eq!(unity_root(&two, 0, false), 0, "根の子が 2 つなら畳まない");
    }

    #[test]
    fn paths_go_down_by_names_and_same_named_siblings_are_ambiguous() {
        let r = rig(false);
        assert_eq!(resolve(&r, 0, ""), Ok(0));
        assert_eq!(resolve(&r, 0, "Armature/Hips/Spine"), Ok(3));
        assert_eq!(resolve(&r, 0, "Body"), Ok(6));
        assert_eq!(resolve(&r, 0, "Armature/Nope"), Err(PathMiss::NotFound));
        assert_eq!(resolve(&r, 0, "Hips"), Err(PathMiss::NotFound));
        assert_eq!(resolve(&r, 0, "Armature/Twin"), Err(PathMiss::Ambiguous));
        assert_eq!(
            resolve(&r, 0, "Armature/Twin 1"),
            Err(PathMiss::Ambiguous),
            "Unity が付け直した名前"
        );
        assert_eq!(
            resolve(&r, 0, "Armature/Hips 1"),
            Err(PathMiss::NotFound),
            "兄弟が 1 つなら付け直さない"
        );
        assert_eq!(resolve(&r, 0, "Armature/Twin x"), Err(PathMiss::NotFound));
        assert_eq!(renamed_base("Twin 12"), Some("Twin"));
        assert_eq!(renamed_base("Twin"), None);
        assert_eq!(renamed_base(" 1"), None);
        // 畳んだ根からは Armature が消える
        let c = rig(true);
        assert_eq!(resolve(&c, 1, "Hips/Spine"), Ok(3));
        assert_eq!(resolve(&c, 1, "Body"), Ok(6));
        assert_eq!(path_of(&c, 1, 3).as_deref(), Some("Hips/Spine"));
        assert_eq!(path_of(&c, 1, 1).as_deref(), Some(""));
        assert_eq!(path_of(&c, 1, 0), None, "Unity の根の外");
    }

    fn request(bones: Vec<BoneValue>) -> Request {
        let text = r#"{"format":1,"kind":"open","id":"r1","target":{"key":"k"},
            "models":[{"id":7,"fbx":"/x/a.fbx"}],
            "renderers":[{"path":"Body","model":7,"node":"Body","blend_shapes":{"smile":40,"unknown":10}}]}"#;
        let mut r = yolu_protocol::files::read_request(text.as_bytes()).unwrap();
        r.bones = bones;
        r
    }

    fn value(node: &str, t: [f32; 3]) -> BoneValue {
        BoneValue {
            model: 7,
            node: node.into(),
            local: Local {
                t,
                r: [0.0, 0.0, 0.0, 2.0],
                s: [1.0; 3],
            },
        }
    }

    #[test]
    fn a_request_becomes_a_pose_and_back() {
        let r = rig(false);
        let layout = Layout {
            parts: vec![PartLayout {
                model: 7,
                bones: 0..r.bones().len(),
                unity_root: 0,
            }],
            renderers: vec![Some(0)],
        };
        let req = request(vec![
            value("Armature/Hips", [0.0, 1.0, 0.0]),
            value("Armature/Twin", [9.0; 3]),
            value("Armature/Twin", [9.0; 3]),
            value("Gone", [9.0; 3]),
        ]);
        let (pose, problems) = layout.pose(&r, &req);
        assert_eq!(pose.locals[2].translation, Vec3::Y);
        assert_eq!(pose.locals[2].rotation, Quat::IDENTITY, "長さ 1 に直す");
        assert_eq!(pose.blend_weights[0], [40.0, 0.0], "知らない名前は飛ばす");
        assert_eq!(
            problems,
            [
                Problem::new("Armature/Twin", Reason::AmbiguousBone),
                Problem::new("Gone", Reason::BoneNotFound),
            ]
        );
        // 戻すと、休みと違う骨と全部の BlendShape の重み
        let mut back = req.clone();
        assert!(layout.write_pose(&r, &pose, &mut back).is_empty());
        assert_eq!(back.bones.len(), 1);
        assert_eq!(back.bones[0].node, "Armature/Hips");
        assert_eq!(back.renderers[0].blend_shapes.len(), 2);
        assert_eq!(back.renderers[0].blend_shapes["blink"], 0.0);
        let (again, none) = layout.pose(&r, &back);
        assert_eq!(again, pose);
        assert!(none.is_empty());
    }

    #[test]
    fn bones_that_a_path_cannot_point_to_are_returned_instead_of_dropped() {
        let r = rig(true);
        // Unity の根は Armature（骨 1）。根の骨 0 はその外、Twin は同じ名前の兄弟
        let layout = Layout {
            parts: vec![PartLayout {
                model: 7,
                bones: 0..r.bones().len(),
                unity_root: 1,
            }],
            renderers: vec![Some(0)],
        };
        let mut pose = r.rest_pose();
        for bone in [0usize, 2, 4] {
            pose.locals[bone].translation = Vec3::new(0.0, 1.0, 0.0);
        }
        let mut back = request(Vec::new());
        let unsaved = layout.write_pose(&r, &pose, &mut back);
        assert_eq!(
            unsaved,
            ["file", "Twin"],
            "根の外の骨と、兄弟と同じ名前の骨"
        );
        assert_eq!(back.bones.len(), 1, "Hips だけが道で指せる");
        assert_eq!(back.bones[0].node, "Hips");
    }
}
