//! ボーンの影響で面を選ぶ式と、ボーンの名前の道（根からの名前の並び）。
//!
//! - 面の重み: 面の 3 頂点が、選んだ骨の組に掛けているウェイトの平均（0〜1）。頂点のウェイトは作るときに和を 1 に揃えてあり、
//!   ウェイトの無い頂点は `Skin::fallback` の関節に固く付く（重み 1）。面の番号は `model_triangles` と同じ通し番号
//!   （メッシュの順 × サブメッシュの順）。しきい値で面を選ぶのは呼ぶ側（重みがしきい値以上で、0 より大きい面）。
//! - 名前の道: 根から骨までの名前の並び。別のファイルのモデルへ骨の組を持ち越すとき、名前だけでなく上の骨の並びも合う骨にだけ
//!   対応させる（同じ名前の骨が別の枝にあっても取り違えない）。同じ親の下に同じ名前の骨が複数あるなど、決まらないときは断る。

use rayon::prelude::*;

use super::Rig;

/// 名前の道が骨に決まらない理由。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BonePathError {
    /// 道が空。
    Empty,
    /// 道の `at` 番目（0 から）の名前の骨が、その上の骨の下に無い。
    NotFound { at: usize },
    /// 道の `at` 番目の名前の骨が、同じ親の下に複数ある。
    Ambiguous { at: usize },
}

impl Rig {
    /// 骨の名前の道（根から、その骨まで）。
    pub fn bone_path(&self, bone: usize) -> Vec<String> {
        let mut path = Vec::new();
        let mut at = Some(bone as u32);
        while let Some(i) = at {
            path.push(self.bones[i as usize].name.clone());
            at = self.bones[i as usize].parent;
        }
        path.reverse();
        path
    }

    /// 名前の道に合う骨（根から順に、名前が合う子を 1 つずつ下る。同じ名前が並ぶときは `Ambiguous`）。
    pub fn resolve_bone_path<S: AsRef<str>>(&self, path: &[S]) -> Result<usize, BonePathError> {
        if path.is_empty() {
            return Err(BonePathError::Empty);
        }
        let mut current: Option<usize> = None;
        for (at, name) in path.iter().enumerate() {
            let name = name.as_ref();
            let candidates: Vec<usize> = match current {
                None => self.roots().collect(),
                Some(b) => self.children(b).iter().map(|&c| c as usize).collect(),
            };
            let mut matching = candidates
                .into_iter()
                .filter(|&c| self.bones[c].name == name);
            let first = matching.next().ok_or(BonePathError::NotFound { at })?;
            if matching.next().is_some() {
                return Err(BonePathError::Ambiguous { at });
            }
            current = Some(first);
        }
        current.ok_or(BonePathError::Empty)
    }

    /// 骨とその子孫の印（骨の数の長さ）。範囲外の骨は全部 false。
    pub fn subtree(&self, bone: usize) -> Vec<bool> {
        let mut out = vec![false; self.bones.len()];
        if bone >= self.bones.len() {
            return out;
        }
        // 親は子より前に並ぶので、前から 1 回で決まる
        out[bone] = true;
        for i in bone + 1..self.bones.len() {
            if let Some(p) = self.bones[i].parent {
                if out[p as usize] {
                    out[i] = true;
                }
            }
        }
        out
    }

    /// 三角形ごとの、選んだ骨（`selected` は骨の数の長さの印）の影響の重み（面の 3 頂点のウェイトの平均）。
    /// 並びは `model_triangles` の三角形の通し番号と同じ。
    pub fn triangle_bone_weights(&self, selected: &[bool]) -> Vec<f32> {
        let mut out = Vec::with_capacity(self.triangle_count());
        for m in &self.meshes {
            let on: Vec<bool> = m
                .skin
                .joints
                .iter()
                .map(|j| selected.get(j.bone as usize).copied().unwrap_or(false))
                .collect();
            let vertices = m.mesh.positions.len();
            let weights: Vec<f32> = (0..vertices)
                .into_par_iter()
                .map(|v| {
                    let list = m.skin.influences_of(v);
                    if list.is_empty() {
                        match m.skin.fallback {
                            Some(f) if on[f as usize] => 1.0,
                            _ => 0.0,
                        }
                    } else {
                        list.iter()
                            .filter(|i| on[i.joint as usize])
                            .map(|i| i.weight)
                            .sum::<f32>()
                            .min(1.0)
                    }
                })
                .collect();
            for s in &m.mesh.submeshes {
                for t in s.indices.chunks_exact(3) {
                    let sum =
                        weights[t[0] as usize] + weights[t[1] as usize] + weights[t[2] as usize];
                    out.push(sum / 3.0);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skin::{demo_figure, FigureDetail};

    fn figure() -> Rig {
        demo_figure(FigureDetail::SMALL)
    }

    fn bone(rig: &Rig, name: &str) -> usize {
        rig.bones().iter().position(|b| b.name == name).unwrap()
    }

    fn mesh_range(rig: &Rig, mesh: usize) -> std::ops::Range<usize> {
        let before: usize = rig.meshes()[..mesh]
            .iter()
            .map(|m| m.mesh.triangle_count())
            .sum();
        before..before + rig.meshes()[mesh].mesh.triangle_count()
    }

    #[test]
    fn a_path_names_the_bone_from_the_root_and_resolves_back() {
        let rig = figure();
        let hand = bone(&rig, "右手");
        let path = rig.bone_path(hand);
        assert_eq!(path.first().map(String::as_str), Some("腰"));
        assert_eq!(path.last().map(String::as_str), Some("右手"));
        assert_eq!(rig.resolve_bone_path(&path), Ok(hand));
        for b in 0..rig.bones().len() {
            assert_eq!(rig.resolve_bone_path(&rig.bone_path(b)), Ok(b), "骨 {b}");
        }
    }

    #[test]
    fn a_path_needs_the_upper_bones_to_match_too() {
        let rig = figure();
        // 名前だけ合う（途中の骨が違う）道は対応させない
        assert_eq!(
            rig.resolve_bone_path(&["腰", "背骨", "右手"]),
            Err(BonePathError::NotFound { at: 2 })
        );
        assert_eq!(
            rig.resolve_bone_path(&["別の根", "背骨"]),
            Err(BonePathError::NotFound { at: 0 })
        );
        assert_eq!(
            rig.resolve_bone_path::<&str>(&[]),
            Err(BonePathError::Empty)
        );
    }

    #[test]
    fn same_named_siblings_are_ambiguous() {
        use crate::skin::{Bone, BoneTransform, RigBudget};
        let bone = |name: &str, parent: Option<u32>| Bone {
            name: name.into(),
            parent,
            rest: BoneTransform::IDENTITY,
        };
        let rig = Rig::new(
            "同名",
            vec![
                bone("根", None),
                bone("枝", Some(0)),
                bone("枝", Some(0)),
                bone("先", Some(1)),
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &RigBudget::default(),
        )
        .unwrap();
        assert_eq!(
            rig.resolve_bone_path(&["根", "枝"]),
            Err(BonePathError::Ambiguous { at: 1 })
        );
        assert_eq!(
            rig.resolve_bone_path(&["根", "枝", "先"]),
            Err(BonePathError::Ambiguous { at: 1 })
        );
        assert_eq!(rig.resolve_bone_path(&["根"]), Ok(0));
    }

    #[test]
    fn a_subtree_is_the_bone_and_all_its_descendants() {
        let rig = figure();
        let upper = bone(&rig, "右上腕");
        let on = rig.subtree(upper);
        let names: Vec<&str> = rig
            .bones()
            .iter()
            .zip(&on)
            .filter(|(_, on)| **on)
            .map(|(b, _)| b.name.as_str())
            .collect();
        assert_eq!(names, ["右上腕", "右前腕", "右手"]);
        assert!(rig.subtree(usize::MAX).iter().all(|on| !on));
        // 根の木は全部
        assert!(rig.subtree(bone(&rig, "腰")).iter().all(|on| *on));
    }

    #[test]
    fn a_rigid_mesh_weighs_one_on_its_bone_and_zero_on_the_others() {
        let rig = figure();
        let head = bone(&rig, "頭");
        let weights = rig.triangle_bone_weights(&rig.subtree(head));
        assert_eq!(weights.len(), rig.triangle_count());
        // 頭は球（メッシュ 5）で、頭の骨に固く付く
        let sphere = mesh_range(&rig, 5);
        assert!(weights[sphere.clone()]
            .iter()
            .all(|w| (*w - 1.0).abs() < 1e-6));
        assert!(
            weights[..sphere.start].iter().all(|w| *w == 0.0),
            "筒は頭の骨に付かない"
        );
        // ほかの骨だけの組では球は 0
        let chest = rig.triangle_bone_weights(&{
            let mut on = vec![false; rig.bones().len()];
            on[bone(&rig, "胸")] = true;
            on
        });
        assert!(chest[sphere].iter().all(|w| *w == 0.0));
    }

    #[test]
    fn a_smooth_tube_blends_weights_toward_the_joint() {
        let rig = figure();
        let hand = bone(&rig, "右手");
        let only_hand = {
            let mut on = vec![false; rig.bones().len()];
            on[hand] = true;
            on
        };
        let weights = rig.triangle_bone_weights(&only_hand);
        // 右腕（メッシュ 1）: 手の側の端は 1 に近く、肩の側は 0、途中は 0 と 1 の間の面がある
        let arm = mesh_range(&rig, 1);
        let slice = &weights[arm];
        assert!(slice.iter().any(|w| *w > 0.99), "手の先は手の骨");
        assert!(
            slice.contains(&0.0),
            "肩のそばは手の骨に掛からない"
        );
        assert!(
            slice.iter().any(|w| *w > 0.0 && *w < 1.0),
            "関節のまわりは混ざる"
        );
        assert!(weights.iter().all(|w| (0.0..=1.0 + 1e-6).contains(w)));
        // 腕全体の木（上腕から下）に広げると、手だけのときより重みは減らない
        let whole = rig.triangle_bone_weights(&rig.subtree(bone(&rig, "右上腕")));
        assert!(weights.iter().zip(&whole).all(|(a, b)| *b + 1e-6 >= *a));
    }

    #[test]
    fn no_bone_selected_weighs_nothing() {
        let rig = figure();
        let weights = rig.triangle_bone_weights(&vec![false; rig.bones().len()]);
        assert!(weights.iter().all(|w| *w == 0.0));
        // 印が短くても、足りない骨は選んでいないものとして読む
        let short = rig.triangle_bone_weights(&[]);
        assert!(short.iter().all(|w| *w == 0.0));
    }
}
