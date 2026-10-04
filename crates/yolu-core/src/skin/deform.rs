//! 線形ブレンドのスキニングと BlendShape（CPU）。メッシュごとに並列、メッシュの中は頂点の塊ごとに並列（rayon）。
//!
//! 1 頂点の式（ufbx の ufbx_get_skin_vertex_matrix と Unity の CPU スキニングと同じ考え方）:
//!   p = バインドの位置 + Σ BlendShape の差分 × 掛け目、n も同じ
//!   M = Σ ウェイト × (骨のワールド × bind_inverse)（ウェイトは作るときに和が 1 に揃えてある）
//!   位置 = M × p、法線 = 長さ 1 に (M の 3 × 3 × n)
//!   ウェイトの無い頂点は fallback の関節の行列（無ければ M = 単位）。

use glam::{Mat3, Mat4, Vec3};
use rayon::prelude::*;

use super::{Rig, RigMesh};
use crate::geometry::ModelMesh;

/// 頂点の塊の大きさ（これより小さいメッシュは 1 本のスレッドで）。
const CHUNK: usize = 4096;

pub(super) fn deform_meshes(rig: &Rig, world: &[Mat4], weights: &[Vec<f32>]) -> Vec<ModelMesh> {
    rig.meshes
        .par_iter()
        .zip(weights.par_iter())
        .map(|(m, w)| deform_mesh(m, world, w))
        .collect()
}

fn deform_mesh(m: &RigMesh, world: &[Mat4], weights: &[f32]) -> ModelMesh {
    let mut positions = m.mesh.positions.clone();
    let mut normals = m.mesh.normals.clone();
    let has_normals = normals.len() == positions.len();
    for (shape, &w) in m.blend_shapes.iter().zip(weights) {
        for (frame, factor) in shape.frame_factors(w) {
            if factor == 0.0 {
                continue;
            }
            let f = &shape.frames[frame];
            for (k, &v) in f.vertices.iter().enumerate() {
                positions[v as usize] += f.positions[k] * factor;
                if has_normals && !f.normals.is_empty() {
                    normals[v as usize] += f.normals[k] * factor;
                }
            }
        }
    }
    let joints: Vec<Mat4> = m
        .skin
        .joints
        .iter()
        .map(|j| world[j.bone as usize] * j.bind_inverse)
        .collect();
    let fallback = m
        .skin
        .fallback
        .map(|f| joints[f as usize])
        .unwrap_or(Mat4::IDENTITY);
    let skin = &m.skin;
    let skin_vertex = |v: usize, p: &mut Vec3, n: Option<&mut Vec3>| {
        let list = skin.influences_of(v);
        let mat = if list.is_empty() {
            fallback
        } else {
            let mut acc = Mat4::ZERO;
            for i in list {
                acc += joints[i.joint as usize] * i.weight;
            }
            acc
        };
        *p = mat.transform_point3(*p);
        if let Some(n) = n {
            *n = (Mat3::from_mat4(mat) * *n).normalize_or_zero();
        }
    };
    if has_normals {
        positions
            .par_chunks_mut(CHUNK)
            .zip(normals.par_chunks_mut(CHUNK))
            .enumerate()
            .for_each(|(c, (ps, ns))| {
                for (k, (p, n)) in ps.iter_mut().zip(ns.iter_mut()).enumerate() {
                    skin_vertex(c * CHUNK + k, p, Some(n));
                }
            });
    } else {
        positions
            .par_chunks_mut(CHUNK)
            .enumerate()
            .for_each(|(c, ps)| {
                for (k, p) in ps.iter_mut().enumerate() {
                    skin_vertex(c * CHUNK + k, p, None);
                }
            });
    }
    ModelMesh {
        name: m.mesh.name.clone(),
        positions,
        normals,
        uvs: m.mesh.uvs.clone(),
        submeshes: m.mesh.submeshes.clone(),
    }
}
