//! 試しの人形（コードで組むスキン。試験・計測・ファイルなしで試すため。実のアバターのデータは使わない）。
//!
//! T ポーズの人形: 腰を根に、背骨・胸・首・頭、肩・上腕・前腕・手（左右）、太もも・すね・足（左右）。胴・腕・脚は骨の鎖に沿った筒
//! （関節のまわりは隣の骨と滑らかに混ぜる）、頭は球（頭の骨に固く付く）。マテリアルは「肌」（胴・腕・脚。UV は島を並べる）と
//! 「顔」（頭）。BlendShape は胴の「おなか」（1 フレーム）と頭の「頭を伸ばす」（中間のフレームつき）。向きは Unity の人型と同じく
//! +Z が前、+X が右。

use glam::{Mat4, Vec2, Vec3};

use super::{
    BlendFrame, BlendShape, Bone, BoneTransform, Influence, Joint, Rig, RigBudget, RigMesh, Skin,
};
use crate::geometry::{cube_sphere, ModelMesh, Submesh};

/// 人形の細かさ。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FigureDetail {
    /// 筒のまわりの分け数。
    pub sides: u32,
    /// 筒の輪の間隔（メートル）。
    pub ring_spacing: f32,
    /// 頭の球の分け数（三角形 12·n²）。
    pub head: u32,
}

impl FigureDetail {
    /// 小さい人形（試験用。三角形 2 千ほど）。
    pub const SMALL: FigureDetail = FigureDetail {
        sides: 12,
        ring_spacing: 0.04,
        head: 4,
    };
    /// 7 万三角形ほど（計測用。Unity 版で測った実のアバターの 69,535 三角形に近い数）。
    pub const AVATAR: FigureDetail = FigureDetail {
        sides: 52,
        ring_spacing: 0.0062,
        head: 24,
    };
}

struct Chain<'a> {
    name: &'a str,
    /// 鎖の骨（根の側から）と、その始まりの位置。最後の骨の終わりは end。
    bones: Vec<(u32, Vec3)>,
    end: Vec3,
    radius: f32,
}

/// 試しの人形。
pub fn demo_figure(detail: FigureDetail) -> Rig {
    let mut bones: Vec<Bone> = Vec::new();
    let mut positions: Vec<Vec3> = Vec::new();
    let mut add = |name: &str, parent: Option<u32>, at: Vec3| -> u32 {
        let base = parent.map(|p| positions[p as usize]).unwrap_or(Vec3::ZERO);
        bones.push(Bone {
            name: name.into(),
            parent,
            rest: BoneTransform {
                translation: at - base,
                ..BoneTransform::IDENTITY
            },
        });
        positions.push(at);
        (bones.len() - 1) as u32
    };
    let hips = add("腰", None, Vec3::new(0.0, 1.0, 0.0));
    let spine = add("背骨", Some(hips), Vec3::new(0.0, 1.12, 0.0));
    let chest = add("胸", Some(spine), Vec3::new(0.0, 1.32, 0.0));
    let neck = add("首", Some(chest), Vec3::new(0.0, 1.55, 0.0));
    let head = add("頭", Some(neck), Vec3::new(0.0, 1.63, 0.0));
    let mut arms = Vec::new();
    let mut legs = Vec::new();
    for (side, sx) in [("右", 1.0f32), ("左", -1.0)] {
        let shoulder = add(
            &format!("{side}肩"),
            Some(chest),
            Vec3::new(0.08 * sx, 1.47, 0.0),
        );
        let upper = add(
            &format!("{side}上腕"),
            Some(shoulder),
            Vec3::new(0.2 * sx, 1.47, 0.0),
        );
        let lower = add(
            &format!("{side}前腕"),
            Some(upper),
            Vec3::new(0.48 * sx, 1.47, 0.0),
        );
        let hand = add(
            &format!("{side}手"),
            Some(lower),
            Vec3::new(0.74 * sx, 1.47, 0.0),
        );
        arms.push((side, sx, [upper, lower, hand]));
        let thigh = add(
            &format!("{side}太もも"),
            Some(hips),
            Vec3::new(0.1 * sx, 0.95, 0.0),
        );
        let shin = add(
            &format!("{side}すね"),
            Some(thigh),
            Vec3::new(0.1 * sx, 0.52, 0.0),
        );
        let foot = add(
            &format!("{side}足"),
            Some(shin),
            Vec3::new(0.1 * sx, 0.1, 0.0),
        );
        legs.push((side, sx, [thigh, shin, foot]));
    }
    let pos = |b: u32| positions[b as usize];
    let mut chains = vec![Chain {
        name: "胴",
        bones: vec![(hips, pos(hips)), (spine, pos(spine)), (chest, pos(chest))],
        end: pos(neck),
        radius: 0.13,
    }];
    let arm_names = ["右腕", "左腕"];
    for (k, (_, sx, [u, l, h])) in arms.iter().enumerate() {
        chains.push(Chain {
            name: arm_names[k],
            bones: vec![(*u, pos(*u)), (*l, pos(*l)), (*h, pos(*h))],
            end: Vec3::new(0.86 * sx, 1.47, 0.0),
            radius: 0.045,
        });
    }
    let leg_names = ["右脚", "左脚"];
    for (k, (_, sx, [t, s, f])) in legs.iter().enumerate() {
        chains.push(Chain {
            name: leg_names[k],
            bones: vec![(*t, pos(*t)), (*s, pos(*s)), (*f, pos(*f))],
            end: Vec3::new(0.1 * sx, 0.02, 0.0),
            radius: 0.06,
        });
    }
    let world_inverse = |b: u32| Mat4::from_translation(-pos(b));
    let mut meshes = Vec::new();
    // 肌の島: 5 本の筒を横に並べる（胴は幅を広めに）
    let islands = [
        (0.0, 0.3),
        (0.3, 0.17),
        (0.47, 0.17),
        (0.64, 0.17),
        (0.81, 0.17),
    ];
    for (i, c) in chains.iter().enumerate() {
        let (u0, w) = islands[i];
        let uv = (Vec2::new(u0 + 0.01, 0.01), Vec2::new(w - 0.02, 0.98));
        meshes.push(tube(c, detail, uv, &world_inverse));
    }
    // 胴の BlendShape「おなか」: 前（+Z）の腹のあたりを法線の向きに膨らませる
    {
        let torso = &mut meshes[0];
        let mut frame = BlendFrame {
            weight: 100.0,
            ..BlendFrame::default()
        };
        for (v, p) in torso.mesh.positions.iter().enumerate() {
            let n = torso.mesh.normals[v];
            let band = 1.0 - ((p.y - 1.15) / 0.12).abs();
            if n.z > 0.2 && band > 0.0 {
                frame.vertices.push(v as u32);
                frame.positions.push(n * (0.05 * band * n.z));
                frame.normals.push(Vec3::ZERO);
            }
        }
        torso.blend_shapes.push(BlendShape {
            name: "おなか".into(),
            frames: vec![frame],
        });
    }
    // 頭: 球、頭の骨に固く付く。BlendShape「頭を伸ばす」は 50 と 100 のフレーム（中間の形）
    let center = pos(head) + Vec3::new(0.0, 0.1, 0.0);
    let mut sphere = cube_sphere(detail.head.max(1), 0.11);
    sphere.name = "頭".into();
    for p in &mut sphere.positions {
        *p += center;
    }
    for s in &mut sphere.submeshes {
        s.material = 1;
    }
    let mut stretch = BlendShape {
        name: "頭を伸ばす".into(),
        frames: Vec::new(),
    };
    for (weight, amount) in [(50.0f32, 0.02f32), (100.0, 0.06)] {
        let mut f = BlendFrame {
            weight,
            ..BlendFrame::default()
        };
        for (v, p) in sphere.positions.iter().enumerate() {
            let up = (p.y - center.y) / 0.11;
            if up > 0.0 {
                f.vertices.push(v as u32);
                f.positions.push(Vec3::new(0.0, amount * up, 0.0));
            }
        }
        stretch.frames.push(f);
    }
    let n = sphere.positions.len();
    meshes.push(RigMesh {
        mesh: sphere,
        skin: Skin::rigid(head, world_inverse(head), n),
        blend_shapes: vec![stretch],
    });
    Rig::new(
        "試しの人形",
        bones,
        meshes,
        vec!["肌".into(), "顔".into()],
        Vec::new(),
        &RigBudget::default(),
    )
    .expect("試しの人形は作れる")
}

/// 骨の鎖に沿った筒（端はふさがない）。継ぎ目の頂点は位置を同じビットにして、UV だけ違う別の頂点にする。
fn tube(
    c: &Chain<'_>,
    detail: FigureDetail,
    (uv_min, uv_size): (Vec2, Vec2),
    world_inverse: &dyn Fn(u32) -> Mat4,
) -> RigMesh {
    let points: Vec<Vec3> = c
        .bones
        .iter()
        .map(|(_, p)| *p)
        .chain(std::iter::once(c.end))
        .collect();
    let lengths: Vec<f32> = points.windows(2).map(|w| (w[1] - w[0]).length()).collect();
    let total: f32 = lengths.iter().sum();
    let rings = ((total / detail.ring_spacing).ceil() as u32 + 1).max(2);
    let sides = detail.sides.max(3);
    let blend = 0.06f32;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut offsets = vec![0u32];
    let mut influences = Vec::new();
    // 関節（鎖の中の骨の境目）の弧長
    let mut joints_at = Vec::new();
    let mut acc = 0.0;
    for l in &lengths[..lengths.len() - 1] {
        acc += l;
        joints_at.push(acc);
    }
    for r in 0..rings {
        let s = total * r as f32 / (rings - 1) as f32;
        // どの区間か
        let (mut seg, mut start) = (0usize, 0.0f32);
        while seg + 1 < lengths.len() && s > start + lengths[seg] {
            start += lengths[seg];
            seg += 1;
        }
        let dir = (points[seg + 1] - points[seg]).normalize();
        let center = points[seg] + dir * (s - start);
        let helper = if dir.y.abs() < 0.9 { Vec3::Y } else { Vec3::Z };
        let u = dir.cross(helper).normalize();
        let v = dir.cross(u);
        // ウェイト（関節のまわり ±blend で隣の骨と滑らかに混ぜる）
        let mut w = vec![0.0f32; c.bones.len()];
        w[seg] = 1.0;
        for (k, &j) in joints_at.iter().enumerate() {
            let d = s - j;
            if d.abs() < blend {
                let t = (d + blend) / (2.0 * blend);
                let t = t * t * (3.0 - 2.0 * t);
                w.iter_mut().for_each(|x| *x = 0.0);
                w[k] = 1.0 - t;
                w[k + 1] = t;
            }
        }
        for j in 0..=sides {
            let a = std::f32::consts::TAU * (j % sides) as f32 / sides as f32;
            let n = u * a.cos() + v * a.sin();
            positions.push(center + n * c.radius);
            normals.push(n);
            uvs.push(uv_min + uv_size * Vec2::new(s / total, j as f32 / sides as f32));
            for (k, &x) in w.iter().enumerate() {
                if x > 0.0 {
                    influences.push(Influence {
                        joint: k as u32,
                        weight: x,
                    });
                }
            }
            offsets.push(influences.len() as u32);
        }
    }
    let row = sides + 1;
    let mut indices = Vec::new();
    for r in 0..rings - 1 {
        for j in 0..sides {
            let a = r * row + j;
            let (b, cc, d) = (a + 1, a + row, a + row + 1);
            // 外から見て時計回り（Unity の表。面の法線 (B − A) × (C − A) が外を向く）
            indices.extend_from_slice(&[a, b, cc, b, d, cc]);
        }
    }
    RigMesh {
        mesh: ModelMesh {
            name: c.name.into(),
            positions,
            normals,
            uvs,
            submeshes: vec![Submesh {
                material: 0,
                indices,
            }],
        },
        skin: Skin {
            joints: c
                .bones
                .iter()
                .map(|(b, _)| Joint {
                    bone: *b,
                    bind_inverse: world_inverse(*b),
                })
                .collect(),
            offsets,
            influences,
            fallback: None,
        },
        blend_shapes: Vec::new(),
    }
}
