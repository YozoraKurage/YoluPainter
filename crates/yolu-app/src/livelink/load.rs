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
//! - 元の絵（Color の流し込み先のスロットの絵）が sRGB の PSD なら、PSD の取り込みと同じ写しの読み（`crate::psd::read_copy`。同じ
//!   スタックの大きさのスレッドで）でレイヤーのまま読む。取り込みが断った PSD（予算・形式）は、今までどおり平らにして読み、断った理由を返す。リニアの PSD は、レイヤーごとに sRGB へ直すと
//!   合成が変わるので、平らにしてから直す。ほかのスロットの PSD はいつも平ら（受けた見た目の絵）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use yolu_core::look::{MissingImage, ReceivedImage};
use yolu_core::skin::{merge, MergePart, MeshPick, Rig, RigBudget};
use yolu_io::psd::{CopyOutcome, CopyRefusal, ImportNote};
use yolu_model::{load_fbx_with, LoadControl, ModelLimits};
use yolu_protocol::files::{Problem, Reason, Request, KEY_NONE};
use yolu_protocol::{MaterialInfo, MaterialKey, TextureProperty};

use super::images::{fit_within, is_psd, linear_to_srgb, read_picture, Picture, PictureError};
use super::layout::{resolve, unity_root, Layout, PartLayout, PathMiss};
use crate::look::link::{MAX_RECEIVED_IMAGE_BYTES, MAX_RECEIVED_SIDE, SHOWN};
use crate::view3d::model::ViewError;

/// 元の絵の画素のバイトの合計の上限（1 つの頼みで読む分。平らな絵の画素と、レイヤーのまま入れる PSD の文書の画素の合計）。
pub const MAX_ORIGINAL_BYTES: u64 = 512 << 20;

/// 1 つの頼みで読んだ元の絵の画素のバイトの合計（上限は [`MAX_ORIGINAL_BYTES`]。試験は小さい上限で確かめる）。
struct OriginalBytes {
    used: u64,
    cap: u64,
}

impl OriginalBytes {
    fn new(cap: u64) -> OriginalBytes {
        OriginalBytes { used: 0, cap }
    }

    /// 上限までの残り。
    fn left(&self) -> u64 {
        self.cap.saturating_sub(self.used)
    }
}

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
pub type Stamp = (std::time::SystemTime, u64);

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

/// 元の絵のファイルの身元（道・更新時刻と大きさ・色の扱い。絵の無いスロットは道が無い）。送り直しで、セットに入れた元の絵のファイルが
/// 変わったかを見る。保証の射程は [`SlotPicture`] と同じ（更新時刻と大きさが同じまま中身だけ変わったファイルは見分けない）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginalSource {
    pub path: Option<String>,
    /// 読めない・無いファイルは None。
    pub stamp: Option<Stamp>,
    pub srgb: bool,
}

/// マテリアル `m` の Color の流し込み先の絵のファイルの身元（今のファイルの更新時刻と大きさを読む）。
pub fn original_source(request: &Request, m: usize) -> OriginalSource {
    let texture = request.materials.get(m).and_then(|mat| {
        mat.textures
            .iter()
            .rfind(|t| SHOWN.iter().any(|(_, p)| *p == t.property))
    });
    match texture.and_then(|t| t.path.as_ref().map(|path| (path, t.srgb))) {
        Some((path, srgb)) => OriginalSource {
            stamp: stamp_of(path),
            path: Some(path.clone()),
            srgb,
        },
        None => OriginalSource::default(),
    }
}

/// 送り直しで元の絵のファイルが変わったかを見るマテリアル（すでにセットのあるもの）。
#[derive(Clone, Debug)]
pub struct Watched {
    /// `materials[]` の番号。
    pub material: usize,
    /// 前に見た元の絵のファイルの身元（覚えていなければ None: 今の身元を覚えるだけ）。
    pub known: Option<OriginalSource>,
    /// セットが元の絵を入れた直後のまま（変わっていれば読み直して入れ直す）。
    pub untouched: bool,
}

/// 元の絵 1 つの結果。
pub enum Original {
    /// 読めた（sRGB の画素へ直した。読んだファイルの大きさ）。
    Picture(Arc<Picture>),
    /// Color の流し込み先の PSD を、レイヤーのまま取り込んだ文書。
    Layers(Box<PsdLayers>),
    /// 絵が無い（Unity の中にしかない絵・テクスチャの無いスロット）: 白で始める。
    White,
    /// 読めない（白で始めて、理由を知らせる）。
    Unreadable { path: String, why: PictureError },
}

/// レイヤーのまま取り込んだ元の絵の PSD（PSD の取り込みと同じ写し。原本は読むだけ）。
pub struct PsdLayers {
    /// 読んだファイル（同じファイルへ書き出すときに確かめるため、取り込んだ場所として覚える）。
    pub path: PathBuf,
    pub doc: yolu_core::Document,
    /// 取り込みの知らせ（無視・落とす・変わる）。
    pub notes: Vec<ImportNote>,
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
    /// 元の絵のファイルが変わったかを見るマテリアル（変わっていて、セットが入れた直後のままなら、元の絵も読む）。
    pub watched: Vec<Watched>,
    /// Rig を組み直さない（送り直しで、FBX・使うメッシュ・マテリアルの付け方が前と同じ）。絵だけを読む。
    pub keep_rig: bool,
    /// レイヤーのまま取り込む元の絵の PSD 1 つの、レイヤーの画素に許すバイト数（設定の「レイヤーのメモリ」。PSD の取り込みと同じ）。
    pub psd_budget: u64,
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
    /// レイヤーのまま取り込めず、平らにして読んだ元の絵の PSD（`materials[]` の番号ごとの、ファイルと断った理由）。
    pub flattened: BTreeMap<usize, (String, CopyRefusal)>,
    /// 元の絵のファイルの身元（読んだ元の絵と、見たマテリアルの。読む前に取った物）。
    pub sources: BTreeMap<usize, OriginalSource>,
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
    // 元の絵のファイルの身元は読む前に取る（読む間にファイルが変わっても、新しい中身に古い身元が付くだけで、次の送り直しで入れ直す）。
    // 見たマテリアルのうち、ファイルが変わっていて、セットが入れた直後のままのものは、元の絵も読む
    let mut sources = BTreeMap::new();
    let mut original_list = inputs.originals.clone();
    for w in &inputs.watched {
        let now = original_source(&request, w.material);
        if w.untouched
            && w.known.as_ref().is_some_and(|k| *k != now)
            && !original_list.contains(&w.material)
        {
            original_list.push(w.material);
        }
        sources.insert(w.material, now);
    }
    for &m in &inputs.originals {
        sources
            .entry(m)
            .or_insert_with(|| original_source(&request, m));
    }
    let mut sizes = BTreeMap::new();
    let Pictures {
        originals,
        flattened,
        slots,
        slot_cache,
    } = read_pictures(
        &request,
        &original_list,
        &inputs.slot_cache,
        inputs.psd_budget,
        MAX_ORIGINAL_BYTES,
        cancel,
        &mut problems,
    )?;
    for (m, o) in &originals {
        match o {
            Original::Picture(p) => {
                sizes.insert(*m, (p.width, p.height));
            }
            Original::Layers(l) => {
                sizes.insert(*m, (l.doc.width(), l.doc.height()));
            }
            _ => {}
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
            flattened,
            sources,
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
    flattened: BTreeMap<usize, (String, CopyRefusal)>,
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
    psd_budget: u64,
    original_cap: u64,
    cancel: &AtomicBool,
    problems: &mut Vec<Problem>,
) -> Result<Pictures, ViewError> {
    let mut out = BTreeMap::new();
    let mut flattened = BTreeMap::new();
    let mut slots: Slots = BTreeMap::new();
    let mut slot_cache: Vec<Arc<SlotPicture>> = Vec::new();
    let mut original_bytes = OriginalBytes::new(original_cap);
    let mut slot_bytes = 0u64;
    let mut seen: Seen = BTreeMap::new();
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
                    Some(path) if t.srgb && is_psd(Path::new(path)) => {
                        let (original, why) = layered_original(
                            path,
                            psd_budget,
                            &mut seen,
                            &mut original_bytes,
                            problems,
                            cancel,
                        )?;
                        if let Some(why) = why {
                            flattened.insert(mi, (path.clone(), why));
                        }
                        original
                    }
                    Some(path) => flat_original(
                        path,
                        t.srgb,
                        &mut seen,
                        &mut original_bytes,
                        problems,
                        cancel,
                    )?,
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
                        None => match read_seen(path, &mut seen, cancel) {
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
        flattened,
        slots,
        slot_cache,
    })
}

/// 同じ頼みの中で読んだ絵（道ごと。同じファイルを読み直さない）。
type Seen = BTreeMap<String, Result<Arc<Picture>, PictureError>>;

/// 絵を読む（`seen` にあれば読み直さない）。
fn read_seen(
    path: &str,
    seen: &mut Seen,
    cancel: &AtomicBool,
) -> Result<Arc<Picture>, PictureError> {
    seen.entry(path.to_owned())
        .or_insert_with(|| read_picture(Path::new(path), cancel).map(Arc::new))
        .clone()
}

/// 元の絵を平らな 1 枚として読む。元の絵の画素の合計（`original_bytes`）が上限を超える絵は読めない物にし、リニアの絵は
/// sRGB の画素へ直す。
fn flat_original(
    path: &str,
    srgb: bool,
    seen: &mut Seen,
    original_bytes: &mut OriginalBytes,
    problems: &mut Vec<Problem>,
    cancel: &AtomicBool,
) -> Result<Original, ViewError> {
    Ok(match read_seen(path, seen, cancel) {
        Ok(p) if p.rgba.len() as u64 <= original_bytes.left() => {
            original_bytes.used += p.rgba.len() as u64;
            let p = if srgb {
                p
            } else {
                let mut q = (*p).clone();
                linear_to_srgb(&mut q.rgba);
                Arc::new(q)
            };
            Original::Picture(p)
        }
        Ok(p) => Original::Unreadable {
            path: path.to_owned(),
            why: PictureError::TooLarge(p.width, p.height),
        },
        Err(PictureError::Cancelled) => return Err(ViewError::Cancelled),
        Err(why) => {
            problems.push(Problem::new(path.to_owned(), Reason::TextureUnreadable));
            Original::Unreadable {
                path: path.to_owned(),
                why,
            }
        }
    })
}

/// 元の絵の PSD をレイヤーのまま読む（PSD の取り込みと同じ写し。`budget` は 1 つの PSD のレイヤーの画素に許すバイト数）。取り込んだ文書の画素は
/// 元の絵の合計（`original_bytes`）に数え、取り込みには、`budget` と合計の残りの小さい方を渡す（残りを超える PSD は読み終える前に断る。
/// 同じ PSD を使うマテリアルが複数あれば、マテリアルごとに別の文書として取り込み、それぞれ数える。断る理由は取り込みの断りのままで、
/// 合計の残りで断ったときも予算の断りになる）。取り込みが断った PSD は平らにして読み、断った理由を一緒に返す。平らにしても読めなければ、
/// 断った理由で読めない物にする（平らの読みの理由より、レイヤーのままを断った理由の方が何が足りないかを言う）。
fn layered_original(
    path: &str,
    budget: u64,
    seen: &mut Seen,
    original_bytes: &mut OriginalBytes,
    problems: &mut Vec<Problem>,
    cancel: &AtomicBool,
) -> Result<(Original, Option<CopyRefusal>), ViewError> {
    use crate::psd::ReadCopyError;
    let granted = budget.min(original_bytes.left());
    let why = match crate::psd::read_copy_on_psd_stack(Path::new(path), granted, cancel) {
        Ok(CopyOutcome::Imported(imported)) => {
            let yolu_io::psd::CopyImport {
                mut document,
                notes,
            } = *imported;
            original_bytes.used = original_bytes
                .used
                .saturating_add(document.allocated_bytes());
            // 文書の予算は、合計の残りではなく、PSD の取り込みと同じ設定の予算にしておく（`granted` 以上なので core は断らない。
            // 入れた後は `sync_budgets` がセットの数と設定から決め直す）
            let _ = document.set_source_budget_bytes(budget);
            let layers = PsdLayers {
                path: PathBuf::from(path),
                doc: document,
                notes,
            };
            return Ok((Original::Layers(Box::new(layers)), None));
        }
        Ok(CopyOutcome::Refused(why)) => why,
        Err(ReadCopyError::Canceled) => return Err(ViewError::Cancelled),
        Err(ReadCopyError::File(e)) => {
            problems.push(Problem::new(path.to_owned(), Reason::TextureUnreadable));
            let why = PictureError::Io(e.to_string());
            let path = path.to_owned();
            return Ok((Original::Unreadable { path, why }, None));
        }
        // 取り込みの中の誤り（core の誤り）: 壊れた PSD と同じく、平らにして読んでみる
        Err(ReadCopyError::Other(e)) => CopyRefusal::Malformed(e.to_string()),
    };
    Ok(
        match flat_original(path, true, seen, original_bytes, problems, cancel)? {
            picture @ Original::Picture(_) => (picture, Some(why)),
            Original::Unreadable { path, .. } => (
                Original::Unreadable {
                    path,
                    why: PictureError::Psd(why),
                },
                None,
            ),
            other => (other, None),
        },
    )
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

    /// Color の流し込み先に PSD 1 つを持つマテリアル 1 つの頼み。
    fn psd_request(path: &Path, srgb: bool) -> Request {
        serde_json::from_value(serde_json::json!({
            "format": 1, "kind": "open", "id": "t", "target": { "key": "k" },
            "models": [], "renderers": [],
            "materials": [ { "key": "object:1", "name": "M", "shader": { "name": "Standard" },
                             "textures": [ { "property": "_MainTex", "path": path.to_string_lossy(),
                                             "srgb": srgb } ] } ]
        }))
        .unwrap()
    }

    /// 4×4 の レイヤー 1 つの PSD。
    fn small_psd() -> Vec<u8> {
        let doc = yolu_io::psd::Document {
            width: 4,
            height: 4,
            layers: vec![yolu_io::psd::Layer {
                id: 1,
                width: 4,
                height: 4,
                pixels_rgba: [10, 20, 30, 255].repeat(16),
                ..yolu_io::psd::Layer::default()
            }],
            composite_rgba: None,
        };
        yolu_io::psd::write(&doc, &yolu_io::psd::Limits::default()).unwrap()
    }

    fn read(request: &Request, budget: u64) -> (Pictures, Vec<Problem>) {
        read_capped(request, &[0], budget, MAX_ORIGINAL_BYTES)
    }

    /// 元の絵を読むマテリアルと、元の絵の合計の上限を指定して読む。
    fn read_capped(
        request: &Request,
        originals: &[usize],
        budget: u64,
        cap: u64,
    ) -> (Pictures, Vec<Problem>) {
        let mut problems = Vec::new();
        let pictures = read_pictures(
            request,
            originals,
            &[],
            budget,
            cap,
            &AtomicBool::new(false),
            &mut problems,
        )
        .unwrap();
        (pictures, problems)
    }

    /// Color の流し込み先に、同じ PSD を持つマテリアル `count` 個の頼み。
    fn shared_psd_request(path: &Path, count: usize) -> Request {
        let material = |i: usize| {
            serde_json::json!({
                "key": format!("object:{i}"), "name": format!("M{i}"), "shader": { "name": "Standard" },
                "textures": [ { "property": "_MainTex", "path": path.to_string_lossy(), "srgb": true } ]
            })
        };
        serde_json::from_value(serde_json::json!({
            "format": 1, "kind": "open", "id": "t", "target": { "key": "k" },
            "models": [], "renderers": [],
            "materials": (0..count).map(material).collect::<Vec<_>>()
        }))
        .unwrap()
    }

    /// 画素が一様でない 32×32 の PSD（レイヤー `layers` 枚。レイヤー 1 枚の画素は 4096 バイト）。
    fn noisy_psd(layers: usize) -> Vec<u8> {
        let layer = |id: i32| yolu_io::psd::Layer {
            id,
            width: 32,
            height: 32,
            pixels_rgba: (0..32 * 32 * 4)
                .map(|i| {
                    (i as u32)
                        .wrapping_mul(2_654_435_761)
                        .wrapping_add(id as u32 * 97) as u8
                        | 1
                })
                .collect(),
            ..yolu_io::psd::Layer::default()
        };
        let doc = yolu_io::psd::Document {
            width: 32,
            height: 32,
            layers: (1..=layers as i32).map(layer).collect(),
            composite_rgba: None,
        };
        yolu_io::psd::write(&doc, &yolu_io::psd::Limits::default()).unwrap()
    }

    #[test]
    fn a_psd_original_is_read_as_layers_and_flattened_only_when_the_layers_are_refused() {
        let dir = std::env::temp_dir().join(format!("yolu-ll-psd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Body.PSD");
        std::fs::write(&path, small_psd()).unwrap();
        // レイヤーのまま（拡張子の大文字小文字は問わない）
        let (p, problems) = read(&psd_request(&path, true), 64 << 20);
        assert!(
            matches!(p.originals.get(&0), Some(Original::Layers(l)) if l.doc.width() == 4 && l.path == path)
        );
        assert!(p.flattened.is_empty() && problems.is_empty());
        // 予算が足りずレイヤーのままを断ったら、平らにして読み、断った理由を返す
        let (p, _) = read(&psd_request(&path, true), 16);
        assert!(matches!(p.originals.get(&0), Some(Original::Picture(pic)) if pic.width == 4));
        assert!(
            matches!(p.flattened.get(&0), Some((_, why)) if why.raised_by_budget()),
            "予算の理由"
        );
        // リニアの PSD は平らにして直す（断った理由は無い）
        let (p, _) = read(&psd_request(&path, false), 64 << 20);
        assert!(matches!(p.originals.get(&0), Some(Original::Picture(_))));
        assert!(p.flattened.is_empty());
        // 平らにしても読めない（CMYK）なら、レイヤーのままを断った理由で読めない物にする
        let mut cmyk = small_psd();
        cmyk[24..26].copy_from_slice(&4u16.to_be_bytes());
        let path = dir.join("cmyk.psd");
        std::fs::write(&path, cmyk).unwrap();
        let (p, problems) = read(&psd_request(&path, true), 64 << 20);
        assert!(
            matches!(
                p.originals.get(&0),
                Some(Original::Unreadable {
                    why: PictureError::Psd(CopyRefusal::ColorFormat { mode: 4, .. }),
                    ..
                })
            ),
            "CMYK の理由"
        );
        assert_eq!(
            problems
                .iter()
                .map(|p| p.known_reason())
                .collect::<Vec<_>>(),
            [Some(Reason::TextureUnreadable)]
        );
        // 無いファイルは読めない物（平らにも読み直さない）
        let (p, _) = read(&psd_request(&dir.join("none.psd"), true), 64 << 20);
        assert!(matches!(
            p.originals.get(&0),
            Some(Original::Unreadable {
                why: PictureError::Io(_),
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
    /// 1 つの頼みで読む元の絵の合計の上限は、レイヤーのまま取り込んだ文書の画素も数える。同じ PSD を使うマテリアルが複数あれば、マテリアルごとに
    /// 別の文書として取り込み、それぞれ数える。残りを超える PSD は平らにし（平らな画素も合計に入る）、それも入らなければ読めない物にする。
    #[test]
    fn layered_psd_originals_count_toward_the_total_original_bytes() {
        let dir = std::env::temp_dir().join(format!("yolu-ll-psd-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("two.psd");
        std::fs::write(&path, noisy_psd(2)).unwrap();
        let request = shared_psd_request(&path, 3);
        let big = 64 << 20;
        // 上限が十分なら、3 つとも別の文書のレイヤーのまま（画素の予算は設定の予算のまま）
        let (p, _) = read_capped(&request, &[0, 1, 2], big, 1 << 30);
        let docs: Vec<&PsdLayers> = (0..3)
            .map(|m| match p.originals.get(&m) {
                Some(Original::Layers(l)) => &**l,
                _ => panic!("{m} はレイヤーのまま"),
            })
            .collect();
        let layered = docs[0].doc.allocated_bytes();
        let flat = 32 * 32 * 4;
        assert!(layered > flat, "2 レイヤー分の画素は平らな 1 枚より大きい");
        assert!(docs.iter().all(|l| l.doc.allocated_bytes() == layered));
        assert!(
            docs.iter().all(|l| l.doc.source_budget_bytes() == big),
            "文書の予算は設定の予算（合計の残りではない）"
        );
        assert!(p.flattened.is_empty());
        // 2 つ分に平らな 1 枚を足した上限: 3 つ目はレイヤーのままだと残りを超えるので断り、平らにして入れる
        let (p, _) = read_capped(&request, &[0, 1, 2], big, 2 * layered + flat);
        assert!(matches!(p.originals.get(&0), Some(Original::Layers(_))));
        assert!(matches!(p.originals.get(&1), Some(Original::Layers(_))));
        assert!(
            matches!(p.originals.get(&2), Some(Original::Picture(pic)) if pic.rgba.len() as u64 == flat),
            "3 つ目は平ら"
        );
        assert!(
            matches!(p.flattened.get(&2), Some((_, why)) if why.raised_by_budget()),
            "予算の断り"
        );
        assert_eq!(p.flattened.len(), 1);
        // 上限がちょうど 1 つ分なら、2 つ目はレイヤーのままも平らも入らない（レイヤーのままを断った理由で読めない物。白で入る）
        let (p, _) = read_capped(&request, &[0, 1], big, layered);
        assert!(matches!(p.originals.get(&0), Some(Original::Layers(_))));
        assert!(matches!(
            p.originals.get(&1),
            Some(Original::Unreadable {
                why: PictureError::Psd(_),
                ..
            })
        ));
        let _ = std::fs::remove_dir_all(dir);
    }
}
