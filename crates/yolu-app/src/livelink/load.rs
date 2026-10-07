//! 頼みを当てる前の、裏の仕事: FBX を読み（読んだことのある道と guid の FBX は読み直さない）、使うメッシュを選んで 1 つの `Rig` に並べ、
//! 元の絵とスロットの絵のファイルを読む。画面のスレッドは、終わった結果を入れるだけ（`super::LiveLink`）。
//!
//! - FBX ごとの倍率: Unity の単位 = yolu-model のメートル × `global_scale` ×（`use_file_scale` なら 1、切っていれば 1 ÷ ファイルの単位の
//!   メートル）。Unity 2022.3 の取り込みと同じ（試験の ASCII の FBX の m・cm で、Unity の骨の localPosition と比べて確かめた）。
//! - `bake_axis_conversion` の FBX は合わせられないので、その FBX のレンダラーを `unsupported_import` にして入れない。
//! - レンダラーのメッシュは、Unity の根（`layout::unity_root`）から `node` の道で引いた骨に付いたメッシュ。サブメッシュは Unity と同じ並び
//!   （FBX のノードのマテリアルのスロットの順・面の無いスロットは飛ばす）なので、`materials` をその順に当てる。足りないサブメッシュと
//!   鍵 `none` のマテリアルは、マテリアルなしの組（1 つにまとめる）。
//! - 骨の名前の道が FBX をまたいで重ならないよう、FBX が 2 つ以上なら根の骨の名前の頭に FBX の番号を付ける。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use yolu_core::look::{MissingImage, ReceivedImage};
use yolu_core::skin::{merge, MergePart, MeshPick, Rig, RigBudget};
use yolu_model::{load_fbx_with, LoadControl, ModelLimits};
use yolu_protocol::files::{Problem, Reason, Request, KEY_NONE};
use yolu_protocol::{MaterialInfo, MaterialKey, TextureProperty};

use super::images::{fit_within, linear_to_srgb, read_picture, Picture, PictureError};
use super::layout::{resolve, unity_root, Layout, PartLayout, PathMiss};
use crate::look::link::{MAX_RECEIVED_IMAGE_BYTES, MAX_RECEIVED_SIDE, SHOWN};
use crate::view3d::model::ViewError;

/// 元の絵の画素のバイトの合計の上限（1 つの頼みで読む分）。
pub const MAX_ORIGINAL_BYTES: u64 = 512 << 20;

/// 読んだ FBX（道と guid で引く。取り込みの設定は並べるときに当てるので、読みには入らない）。
#[derive(Debug)]
pub struct LoadedFbx {
    pub path: String,
    pub guid: String,
    pub rig: Rig,
    /// ファイルの単位（1 単位が何メートルか）。
    pub unit_meters: f64,
    pub warnings: Vec<String>,
}

/// ファイルの身元（更新時刻と大きさ。同じなら中身が同じとみなす）。
type Stamp = (std::time::SystemTime, u64);

fn stamp_of(path: &str) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// 読んだスロットの絵（復号して 2048 までに縮めた物）。道・更新時刻・大きさ・色の扱いが同じなら、送り直しで読み直さずに使い回す。
/// 保証の射程: 更新時刻と大きさが同じまま中身だけ変わったファイルは見分けない（ファイルシステムの時刻の細かさの内）。
#[derive(Debug)]
pub struct SlotPicture {
    path: String,
    stamp: Stamp,
    srgb: bool,
    pub image: Arc<ReceivedImage>,
}

/// 元の絵 1 つの結果。
#[derive(Clone, Debug)]
pub enum Original {
    /// 読めた（sRGB の画素へ直した。読んだファイルの大きさ）。
    Picture(Arc<Picture>),
    /// 絵が無い（Unity の中にしかない絵・テクスチャの無いスロット）: 白で始める。
    White,
    /// 読めない（白で始めて、理由を知らせる）。
    Unreadable { path: String, why: PictureError },
}

/// 裏の仕事に渡すもの。
pub struct Inputs {
    pub request: Arc<Request>,
    /// 前に読んだ FBX（同じ道と guid なら読み直さない）。
    pub cache: Vec<Arc<LoadedFbx>>,
    /// 前に読んだスロットの絵（同じファイルなら読み直さない）。
    pub slot_cache: Vec<Arc<SlotPicture>>,
    /// 元の絵を読むマテリアル（`materials[]` の番号）。
    pub originals: Vec<usize>,
    /// Rig を組み直さない（送り直しで、FBX・使うメッシュ・マテリアルの付け方が前と同じ）。絵だけを読む。
    pub keep_rig: bool,
}

/// 裏の仕事の結果（画面のスレッドが入れる）。
pub struct Opened {
    pub request: Arc<Request>,
    pub fbx: Vec<Arc<LoadedFbx>>,
    pub layout: Layout,
    /// テクスチャセットを結ぶマテリアル（`materials[]` の並び。足りないサブメッシュがあれば、最後にマテリアルなしの組）。
    pub materials: Vec<MaterialInfo>,
    /// 元の絵（`materials[]` の番号ごと）。
    pub originals: BTreeMap<usize, Original>,
    /// スロットの絵（`materials[]` の番号・スロット）。読めない・予算を超える・ファイルの無い絵は理由。
    pub slots: BTreeMap<(usize, String), Result<Arc<ReceivedImage>, MissingImage>>,
    /// 今の頼みのスロットの絵のうち、次の送り直しで使い回せる物（今の頼みが使う物だけ）。
    pub slot_cache: Vec<Arc<SlotPicture>>,
    pub problems: Vec<Problem>,
    /// 何も使えるレンダラーが無い（返事は断り）。
    pub nothing: bool,
}

/// Unity の鍵を、マテリアルの鍵にする: アセットの `guid:<32 桁>/fileid:<数>` は識別子つき、マテリアルの無いサブメッシュの `none` は
/// マテリアルなしの組、ほか（シーンの中のマテリアルの `object:…`・`instance:…`、形の違う鍵）は名前だけの鍵。
pub fn material_key(key: &str, name: &str) -> MaterialKey {
    if key == KEY_NONE {
        return MaterialKey::Unassigned;
    }
    let asset = key.strip_prefix("guid:").and_then(|rest| {
        let (guid, file) = rest.split_once("/fileid:")?;
        let ok = guid.len() == 32
            && guid
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        Some((ok.then(|| guid.to_owned())?, file.parse::<i64>().ok()?))
    });
    MaterialKey::Material {
        name: crate::model::key_name(name),
        asset,
    }
}

/// 頼みのマテリアルを、テクスチャセットを結ぶ記録にする。Color の流し込み先は `_MainTex`（[`SHOWN`]）。元の絵の大きさが分かれば
/// テクスチャの項目に入れる（新しいセットの大きさに使う）。
pub fn material_infos(request: &Request, sizes: &BTreeMap<usize, (u32, u32)>) -> Vec<MaterialInfo> {
    request
        .materials
        .iter()
        .enumerate()
        .map(|(i, m)| (i, m, material_key(&m.key, &m.name)))
        .map(|(i, m, key)| MaterialInfo {
            routes: if key == MaterialKey::Unassigned {
                Vec::new()
            } else {
                SHOWN
                    .iter()
                    .map(|(c, p)| yolu_protocol::ChannelRoute {
                        channel: c.index() as u8,
                        property: (*p).to_owned(),
                    })
                    .collect()
            },
            key,
            shader: m.shader.name.clone(),
            textures: m
                .textures
                .iter()
                .map(|t| {
                    let shown = SHOWN.iter().any(|(_, p)| *p == t.property);
                    let (width, height) = if shown {
                        sizes.get(&i).copied().unwrap_or((0, 0))
                    } else {
                        (0, 0)
                    };
                    TextureProperty {
                        name: t.property.clone(),
                        width,
                        height,
                    }
                })
                .collect(),
        })
        .collect()
}

/// FBX の倍率（Unity の単位 ÷ yolu-model のメートル）。
pub fn import_scale(import: &yolu_protocol::files::ImportSettings, unit_meters: f64) -> f32 {
    let file = if import.use_file_scale || !(unit_meters.is_finite() && unit_meters > 0.0) {
        1.0
    } else {
        1.0 / unit_meters
    };
    (import.global_scale * file) as f32
}

fn load_one(path: &str, cancel: &AtomicBool) -> Result<LoadedFbx, ViewError> {
    let loaded = load_fbx_with(
        Path::new(path),
        &ModelLimits::default(),
        LoadControl {
            cancel: Some(cancel),
            progress: None,
        },
    )?;
    Ok(LoadedFbx {
        path: path.to_owned(),
        guid: String::new(),
        unit_meters: loaded.report.file_unit_meters,
        warnings: loaded.report.warnings,
        rig: loaded.rig,
    })
}

/// `models[]` ごとの読んだ FBX（軸を焼いた・読めない FBX は None）と、読めなかった理由。
type ReadFbx = (Vec<Option<Arc<LoadedFbx>>>, Vec<Problem>);

/// 使う FBX を読む（前に読んだ物は使い回す）。読めない FBX は理由を返す。
fn read_fbx(
    request: &Request,
    cache: &[Arc<LoadedFbx>],
    cancel: &AtomicBool,
) -> Result<ReadFbx, ViewError> {
    let mut out: Vec<Option<Arc<LoadedFbx>>> = Vec::with_capacity(request.models.len());
    let mut fresh: Vec<Arc<LoadedFbx>> = Vec::new();
    let mut problems = Vec::new();
    for model in &request.models {
        if cancel.load(Ordering::Relaxed) {
            return Err(ViewError::Cancelled);
        }
        if model.import.bake_axis_conversion {
            out.push(None);
            continue;
        }
        let same = |f: &&Arc<LoadedFbx>| f.path == model.fbx && f.guid == model.guid;
        if let Some(found) = cache.iter().chain(&fresh).find(same) {
            out.push(Some(found.clone()));
            continue;
        }
        match load_one(&model.fbx, cancel) {
            Ok(mut f) => {
                f.guid.clone_from(&model.guid);
                let f = Arc::new(f);
                fresh.push(f.clone());
                out.push(Some(f));
            }
            Err(ViewError::Cancelled) => return Err(ViewError::Cancelled),
            Err(_) => {
                problems.push(Problem::new(model.fbx.clone(), Reason::FbxUnreadable));
                out.push(None);
            }
        }
    }
    Ok((out, problems))
}

/// 裏の仕事の中身: FBX を読み、並べ、絵を読む。
pub fn run(
    inputs: Inputs,
    cancel: &AtomicBool,
) -> Result<(Option<Rig>, Vec<String>, Opened), ViewError> {
    let request = inputs.request.clone();
    let (fbx, mut problems) = read_fbx(&request, &inputs.cache, cancel)?;
    // レンダラーごとに、FBX の中のメッシュ（部分の Rig の番号）を引く
    let mut picks: Vec<Option<(usize, usize)>> = vec![None; request.renderers.len()];
    for (ri, r) in request.renderers.iter().enumerate() {
        let Some(mi) = request.model_index(r.model) else {
            continue;
        };
        let model = &request.models[mi];
        let Some(f) = &fbx[mi] else {
            if model.import.bake_axis_conversion {
                problems.push(Problem::new(r.path.clone(), Reason::UnsupportedImport));
            }
            continue;
        };
        if !r.enabled {
            continue;
        }
        let root = unity_root(&f.rig, 0, model.import.preserve_hierarchy);
        let found = resolve(&f.rig, root, &r.node).and_then(|bone| {
            f.rig
                .meshes()
                .iter()
                .position(|m| m.node == Some(bone as u32))
                .ok_or(PathMiss::NotFound)
        });
        match found {
            Ok(mesh) => picks[ri] = Some((mi, mesh)),
            Err(miss) => problems.push(Problem::new(r.path.clone(), miss.reason())),
        }
    }
    let nothing = picks.iter().all(Option::is_none);
    // マテリアル（足りないサブメッシュは、頼みのマテリアルなしの組 `none`。無ければ最後にマテリアルなしの組を追加する）
    let given_none = request.materials.iter().position(|m| m.key == KEY_NONE);
    let unassigned = given_none.unwrap_or(request.materials.len()) as i32;
    let mut needs_unassigned = false;
    let mut parts: Vec<(usize, Vec<MeshPick>)> = Vec::new();
    let mut renderer_slot: Vec<Option<(usize, usize)>> = vec![None; request.renderers.len()];
    for (mi, f) in fbx.iter().enumerate() {
        let Some(f) = f else { continue };
        let mut meshes = Vec::new();
        for (ri, pick) in picks.iter().enumerate() {
            let Some((pm, mesh)) = *pick else { continue };
            if pm != mi || meshes.iter().any(|p: &MeshPick| p.mesh == mesh) {
                continue;
            }
            let subs = f.rig.meshes()[mesh].mesh.submeshes.len();
            let given = &request.renderers[ri].materials;
            let materials: Vec<i32> = (0..subs)
                .map(|k| match given.get(k) {
                    Some(&m) => m as i32,
                    None => {
                        needs_unassigned |= given_none.is_none();
                        unassigned
                    }
                })
                .collect();
            renderer_slot[ri] = Some((parts.len(), meshes.len()));
            meshes.push(MeshPick { mesh, materials });
        }
        if !meshes.is_empty()
            || request
                .bones
                .iter()
                .any(|b| b.model == request.models[mi].id)
        {
            parts.push((mi, meshes));
        }
    }
    let mut sizes = BTreeMap::new();
    let Pictures {
        originals,
        slots,
        slot_cache,
    } = read_pictures(
        &request,
        &inputs.originals,
        &inputs.slot_cache,
        cancel,
        &mut problems,
    )?;
    for (m, o) in &originals {
        if let Original::Picture(p) = o {
            sizes.insert(*m, (p.width, p.height));
        }
    }
    let mut materials = material_infos(&request, &sizes);
    if needs_unassigned {
        materials.push(MaterialInfo {
            key: MaterialKey::Unassigned,
            shader: String::new(),
            textures: Vec::new(),
            routes: Vec::new(),
        });
    }
    let mut warnings = Vec::new();
    let mut rig = None;
    let mut layout = Layout::default();
    if !inputs.keep_rig && !nothing {
        let several = parts.len() > 1;
        let merge_parts: Vec<MergePart<'_>> = parts
            .iter()
            .map(|(mi, meshes)| {
                let f = fbx[*mi].as_ref().expect("読めた FBX だけ");
                let model = &request.models[*mi];
                MergePart {
                    rig: &f.rig,
                    meshes: meshes.clone(),
                    blend_shapes: model.import.import_blend_shapes,
                    scale: import_scale(&model.import, f.unit_meters),
                    root_name: several.then(|| format!("{}:{}", model.id, f.rig.name())),
                }
            })
            .collect();
        let names: Vec<String> = materials
            .iter()
            .map(|m| match &m.key {
                MaterialKey::Material { name, .. } => name.clone(),
                MaterialKey::Unassigned => crate::model::NO_MATERIAL.to_owned(),
            })
            .collect();
        let name = if request.target.name.trim().is_empty() {
            "Unity"
        } else {
            request.target.name.as_str()
        };
        let merged = merge(name, &merge_parts, Some(names), &RigBudget::default())?;
        for (k, ((mi, _), range)) in parts.iter().zip(&merged.parts).enumerate() {
            let f = fbx[*mi].as_ref().expect("読めた FBX だけ");
            let model = &request.models[*mi];
            let local_root = unity_root(&f.rig, 0, model.import.preserve_hierarchy);
            layout.parts.push(PartLayout {
                model: model.id,
                bones: range.bones.clone(),
                unity_root: range.bones.start + local_root,
            });
            warnings.extend(f.warnings.iter().cloned());
            let _ = k;
        }
        layout.renderers = renderer_slot
            .iter()
            .map(|slot| slot.map(|(part, mesh)| merged.parts[part].meshes.start + mesh))
            .collect();
        rig = Some(merged.rig);
    }
    let fbx_used: Vec<Arc<LoadedFbx>> = fbx.into_iter().flatten().collect();
    Ok((
        rig,
        warnings,
        Opened {
            request,
            fbx: fbx_used,
            layout,
            materials,
            originals,
            slots,
            slot_cache,
            problems,
            nothing,
        },
    ))
}

type Slots = BTreeMap<(usize, String), Result<Arc<ReceivedImage>, MissingImage>>;

/// 絵を読んだ結果。
struct Pictures {
    originals: BTreeMap<usize, Original>,
    slots: Slots,
    /// 次の送り直しで使い回せるスロットの絵（今の頼みが使う物だけ）。
    slot_cache: Vec<Arc<SlotPicture>>,
}

/// 元の絵（`originals` のマテリアルの、Color の流し込み先のスロット）と、lilToon のスロットの絵（ほかのスロット）を読む。スロットの絵は、
/// `cache` に同じファイル（道・更新時刻・大きさ・色の扱い）があれば読み直さない。
fn read_pictures(
    request: &Request,
    originals: &[usize],
    cache: &[Arc<SlotPicture>],
    cancel: &AtomicBool,
    problems: &mut Vec<Problem>,
) -> Result<Pictures, ViewError> {
    let mut out = BTreeMap::new();
    let mut slots: Slots = BTreeMap::new();
    let mut slot_cache: Vec<Arc<SlotPicture>> = Vec::new();
    let mut original_bytes = 0u64;
    let mut slot_bytes = 0u64;
    let mut seen: BTreeMap<String, Result<Arc<Picture>, PictureError>> = BTreeMap::new();
    let read = |path: &str, seen: &mut BTreeMap<String, Result<Arc<Picture>, PictureError>>| {
        seen.entry(path.to_owned())
            .or_insert_with(|| read_picture(Path::new(path), cancel).map(Arc::new))
            .clone()
    };
    for (mi, m) in request.materials.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(ViewError::Cancelled);
        }
        let liltoon = crate::look::link::is_liltoon(m);
        for t in &m.textures {
            let shown = SHOWN.iter().any(|(_, p)| *p == t.property);
            if shown {
                if !originals.contains(&mi) {
                    continue;
                }
                let original = match &t.path {
                    None => Original::White,
                    Some(path) => match read(path, &mut seen) {
                        Ok(p) if original_bytes + p.rgba.len() as u64 <= MAX_ORIGINAL_BYTES => {
                            original_bytes += p.rgba.len() as u64;
                            let p = if t.srgb {
                                p
                            } else {
                                let mut q = (*p).clone();
                                linear_to_srgb(&mut q.rgba);
                                Arc::new(q)
                            };
                            Original::Picture(p)
                        }
                        Ok(p) => Original::Unreadable {
                            path: path.clone(),
                            why: PictureError::TooLarge(p.width, p.height),
                        },
                        Err(PictureError::Cancelled) => return Err(ViewError::Cancelled),
                        Err(why) => {
                            problems.push(Problem::new(path.clone(), Reason::TextureUnreadable));
                            Original::Unreadable {
                                path: path.clone(),
                                why,
                            }
                        }
                    },
                };
                out.insert(mi, original);
                continue;
            }
            if !liltoon || crate::look::liltoon::slot(&t.property).is_none() {
                continue;
            }
            let key = (mi, t.property.clone());
            let result = match &t.path {
                None => Err(MissingImage::NotAFile),
                Some(path) => {
                    let srgb = t.srgb && !t.normal_map;
                    // 身元は読む前に取る（読む間にファイルが変わっても、新しい中身に古い身元が付くだけで、次は読み直す）
                    let stamp = stamp_of(path);
                    let hit = stamp.and_then(|stamp| {
                        cache
                            .iter()
                            .find(|c| c.path == *path && c.stamp == stamp && c.srgb == srgb)
                            .cloned()
                    });
                    let loaded = match hit {
                        Some(c) => Ok(c.image.clone()),
                        None => match read(path, &mut seen) {
                            Ok(p) => {
                                let p = fit_within((*p).clone(), MAX_RECEIVED_SIDE);
                                Ok(Arc::new(ReceivedImage {
                                    width: p.width,
                                    height: p.height,
                                    srgb,
                                    pixels: p.rgba.into(),
                                }))
                            }
                            Err(PictureError::Cancelled) => return Err(ViewError::Cancelled),
                            Err(_) => {
                                problems
                                    .push(Problem::new(path.clone(), Reason::TextureUnreadable));
                                Err(MissingImage::Unreadable)
                            }
                        },
                    };
                    loaded.and_then(|image| {
                        let bytes = image.pixels.len() as u64;
                        if slot_bytes + bytes > MAX_RECEIVED_IMAGE_BYTES {
                            return Err(MissingImage::OverBudget);
                        }
                        slot_bytes += bytes;
                        if let Some(stamp) = stamp.filter(|stamp| {
                            !slot_cache
                                .iter()
                                .any(|c| c.path == *path && c.stamp == *stamp && c.srgb == srgb)
                        }) {
                            slot_cache.push(Arc::new(SlotPicture {
                                path: path.clone(),
                                stamp,
                                srgb,
                                image: image.clone(),
                            }));
                        }
                        Ok(image)
                    })
                }
            };
            slots.insert(key, result);
        }
        // Color の流し込み先のテクスチャの項目が無いマテリアル（絵の無いスロット）は白で始める
        if originals.contains(&mi) && !out.contains_key(&mi) {
            out.insert(mi, Original::White);
        }
    }
    problems.dedup();
    Ok(Pictures {
        originals: out,
        slots,
        slot_cache,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_material_keys_become_set_keys() {
        let asset = material_key(
            "guid:0123456789abcdef0123456789abcdef/fileid:2100000",
            "Skin",
        );
        assert_eq!(
            asset,
            MaterialKey::Material {
                name: "Skin".into(),
                asset: Some(("0123456789abcdef0123456789abcdef".into(), 2_100_000)),
            }
        );
        // シーンの中のマテリアル・GlobalObjectId の無いマテリアル・形の違う鍵は名前だけ
        for key in [
            "object:GlobalObjectId_V1-2-0123-4567-0-77",
            "instance:-1234",
            "guid:XYZ/fileid:1",
        ] {
            assert_eq!(
                material_key(key, "Skin"),
                MaterialKey::Material {
                    name: "Skin".into(),
                    asset: None
                },
                "{key}"
            );
        }
        assert_eq!(material_key(KEY_NONE, ""), MaterialKey::Unassigned);
    }
}
