//! C の関数（Unity の C# から P/Invoke で呼ぶ。C# の宣言は csbindgen がこのファイルから作る）。
//!
//! 決まり:
//! - Unity の主スレッドから呼ぶ前提で、どれも待たない（つなぐ・送る・受けるは裏のスレッド。ここは錠を取って状態を読み書きするだけ）。
//!   画素の写し（ylb_copy_dirty）だけは写す量に比例して時間がかかる。
//! - 返す値: 0 以上は成功、負は失敗（YLB_E_*）。文字列は UTF-8 のバイトと長さ。受け取る文字列の領域が足りなければ、要る長さを返して
//!   入るだけ（文字の途中で切らない）写す。
//! - Unity は一度読んだネイティブの DLL を手放さない。ドメインの読み直しの後も前のつながりが残るので、C# は最初に ylb_disconnect_all を呼ぶ。
//!   この口は小さく保ち、変えるときは関数を足す（今の関数の意味を変えるなら ylb_abi_version を上げ、C# は合わない版を使わない）。
//! - パニックはここで受け止めて YLB_E_PANIC にする（Unity を落とさない）。
//!
//! 安全の決まり（unsafe の関数の全部）: ポインターは null か、渡した長さ（要素の数）のぶん読める（出力は書ける）領域を指すこと。
//! 呼んでいる間に、同じ領域を別のスレッドで書き換えないこと。構造体の出力は 1 つぶんの領域を指すこと。

use std::collections::BTreeMap;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use yolu_protocol::*;

use crate::copy::{copy_dirty, Strip};
use crate::session::{Session, Status};
use crate::testserver::{TestServer, YlbTestServerStats};

/// この口の版。関数の意味・引数・構造体を変えたら上げる（足すだけなら上げない）。
pub const ABI_VERSION: u32 = 1;

pub const YLB_E_HANDLE: i32 = -1;
pub const YLB_E_ARGUMENT: i32 = -2;
pub const YLB_E_STATE: i32 = -3;
pub const YLB_E_PANIC: i32 = -5;
pub const YLB_E_SHM: i32 = -6;

static SESSIONS: Mutex<BTreeMap<u64, Arc<Session>>> = Mutex::new(BTreeMap::new());
static SERVERS: Mutex<BTreeMap<u64, TestServer>> = Mutex::new(BTreeMap::new());
static NEXT_HANDLE: AtomicU64 = AtomicU64::new(1);

fn guard<T>(fallback: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(fallback)
}

fn session(handle: u64) -> Option<Arc<Session>> {
    SESSIONS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&handle)
        .cloned()
}

unsafe fn bytes<'a, T>(ptr: *const T, len: i32) -> Option<&'a [T]> {
    if len < 0 || (len > 0 && ptr.is_null()) {
        return None;
    }
    if len == 0 {
        return Some(&[]);
    }
    Some(std::slice::from_raw_parts(ptr, len as usize))
}

unsafe fn text<'a>(ptr: *const u8, len: i32) -> Option<&'a str> {
    std::str::from_utf8(bytes(ptr, len)?).ok()
}

/// 文字列を領域へ写す（文字の途中で切らない）。要る長さを返す。
unsafe fn put_text(s: &str, buf: *mut u8, cap: i32) -> i32 {
    put_text_written(s, buf, cap);
    s.len() as i32
}

/// 文字列を領域へ写し、写したバイトの数を返す。
unsafe fn put_text_written(s: &str, buf: *mut u8, cap: i32) -> i32 {
    if buf.is_null() || cap <= 0 {
        return 0;
    }
    let mut n = s.len().min(cap as usize);
    while !s.is_char_boundary(n) {
        n -= 1;
    }
    std::ptr::copy_nonoverlapping(s.as_ptr(), buf, n);
    n as i32
}

/// この口の版（最初に呼ぶ。C# の知っている版と違えば、ほかの関数を呼ばない）。
#[no_mangle]
pub extern "C" fn ylb_abi_version() -> u32 {
    ABI_VERSION
}

/// 読めるプロトコルの版（上の 16 ビットが一番古い版、下の 16 ビットが一番新しい版）。
#[no_mangle]
pub extern "C" fn ylb_protocol_versions() -> u32 {
    ((MIN_PROTOCOL_VERSION as u32) << 16) | PROTOCOL_VERSION as u32
}

/// つなぎ始める（すぐに返る。つながったかは ylb_status と知らせで見る）。返すのはつながりの番号（0 は名前が使えない）。
#[no_mangle]
pub unsafe extern "C" fn ylb_connect(
    name: *const u8,
    name_len: i32,
    agent: *const u8,
    agent_len: i32,
) -> u64 {
    guard(0, || {
        let Some(name) = text(name, name_len) else {
            return 0;
        };
        let Some(agent) = text(agent, agent_len) else {
            return 0;
        };
        if !yolu_protocol::link::valid_link_name(name) {
            return 0;
        }
        let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        let s = Session::start(
            id,
            name.to_owned(),
            format!("{agent} (yolu-bridge abi {ABI_VERSION})"),
        );
        SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, s);
        id
    })
}

/// 切る（Bye を送る。共有メモリの写像を手放す）。番号はもう使えない。
#[no_mangle]
pub extern "C" fn ylb_disconnect(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let s = SESSIONS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&handle);
        match s {
            Some(s) => {
                s.close();
                0
            }
            None => YLB_E_HANDLE,
        }
    })
}

/// 全部のつながりを切る（ドメインの読み直しの後に、前のドメインのつながりを片付ける）。切った数を返す。
#[no_mangle]
pub extern "C" fn ylb_disconnect_all() -> i32 {
    guard(YLB_E_PANIC, || {
        let all: Vec<_> = std::mem::take(&mut *SESSIONS.lock().unwrap_or_else(|e| e.into_inner()))
            .into_values()
            .collect();
        for s in &all {
            s.close();
        }
        all.len() as i32
    })
}

/// 状態: 0 = つないでいる、1 = つながった、2 = 閉じた、3 = 失敗。
#[no_mangle]
pub extern "C" fn ylb_status(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || match session(handle) {
        Some(s) => s.status() as i32,
        None => YLB_E_HANDLE,
    })
}

/// 状態の説明（UTF-8）。要る長さを返す。
#[no_mangle]
pub unsafe extern "C" fn ylb_status_text(handle: u64, buf: *mut u8, cap: i32) -> i32 {
    guard(YLB_E_PANIC, || match session(handle) {
        Some(s) => put_text(&s.state().status_text, buf, cap),
        None => YLB_E_HANDLE,
    })
}

/// 何かが変わるたびに増える番号（C# は変わっていなければ何もしない）。
#[no_mangle]
pub extern "C" fn ylb_serial(handle: u64) -> u64 {
    guard(0, || session(handle).map(|s| s.state().serial).unwrap_or(0))
}

/// 知らせ 1 つ。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct YlbEvent {
    /// 1 = つながった、2 = 断られた、3 = 失敗、4 = 閉じた、5 = セットが来た、6 = セットが消えた、7 = 相手からの誤り、8 = こちらの知らせ。
    pub kind: i32,
    pub set: u32,
    pub code: i32,
    /// 説明の UTF-8 の長さ（領域が足りなければ切ってある）。
    pub text_len: i32,
}

/// 知らせを 1 つ取り出す。取り出せば 1、無ければ 0。
#[no_mangle]
pub unsafe extern "C" fn ylb_next_event(
    handle: u64,
    event: *mut YlbEvent,
    buf: *mut u8,
    cap: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if event.is_null() {
            return YLB_E_ARGUMENT;
        }
        let Some(e) = s.state().events.pop_front() else {
            return 0;
        };
        let written = put_text_written(&e.text, buf, cap);
        *event = YlbEvent {
            kind: e.kind as i32,
            set: e.set,
            code: e.code,
            text_len: written,
        };
        1
    })
}

// ───────── モデルを送る（C# が 1 つずつ足し、最後に ylb_model_send） ─────────

/// モデルの組み立てを始める（前の組み立ては捨てる）。
#[no_mangle]
pub unsafe extern "C" fn ylb_model_begin(handle: u64, name: *const u8, name_len: i32) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = text(name, name_len) else {
            return YLB_E_ARGUMENT;
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        b.model = Some(Model {
            generation: 0,
            name: name.to_owned(),
            materials: Vec::new(),
            meshes: Vec::new(),
        });
        0
    })
}

/// マテリアルの組を足す。unassigned が 0 でなければマテリアルの無いスロットの組（名前・GUID は使わない）。guid_len が 0 ならアセットでない。
/// 返すのはマテリアルの番号。
#[no_mangle]
pub unsafe extern "C" fn ylb_model_material(
    handle: u64,
    unassigned: i32,
    name: *const u8,
    name_len: i32,
    guid: *const u8,
    guid_len: i32,
    file_id: i64,
    shader: *const u8,
    shader_len: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let (Some(name), Some(guid), Some(shader)) = (
            text(name, name_len),
            text(guid, guid_len),
            text(shader, shader_len),
        ) else {
            return YLB_E_ARGUMENT;
        };
        if name.len() > MAX_NAME_BYTES
            || shader.len() > MAX_NAME_BYTES
            || (!guid.is_empty() && !is_asset_guid(guid))
        {
            return YLB_E_ARGUMENT;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        if m.materials.len() >= MAX_MATERIALS {
            return YLB_E_ARGUMENT;
        }
        let key = if unassigned != 0 {
            MaterialKey::Unassigned
        } else {
            MaterialKey::Material {
                name: name.to_owned(),
                asset: (!guid.is_empty()).then(|| (guid.to_owned(), file_id)),
            }
        };
        m.materials.push(MaterialInfo {
            key,
            shader: shader.to_owned(),
            textures: Vec::new(),
            routes: Vec::new(),
        });
        (m.materials.len() - 1) as i32
    })
}

/// マテリアルのシェーダーの 2D テクスチャのプロパティと、今入っているテクスチャの大きさ（無ければ 0）を足す。
#[no_mangle]
pub unsafe extern "C" fn ylb_model_material_texture(
    handle: u64,
    material: i32,
    name: *const u8,
    name_len: i32,
    width: u32,
    height: u32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = text(name, name_len) else {
            return YLB_E_ARGUMENT;
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        let Some(mat) = usize::try_from(material)
            .ok()
            .and_then(|i| m.materials.get_mut(i))
        else {
            return YLB_E_ARGUMENT;
        };
        if name.len() > MAX_NAME_BYTES || mat.textures.len() >= MAX_TEXTURE_PROPERTIES {
            return YLB_E_ARGUMENT;
        }
        mat.textures.push(TextureProperty {
            name: name.to_owned(),
            width,
            height,
        });
        0
    })
}

/// Unity 側が見せられるチャンネルと流し込み先のプロパティを足す。
#[no_mangle]
pub unsafe extern "C" fn ylb_model_material_route(
    handle: u64,
    material: i32,
    channel: i32,
    property: *const u8,
    property_len: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(property) = text(property, property_len) else {
            return YLB_E_ARGUMENT;
        };
        if !(0..channel::COUNT as i32).contains(&channel) || property.len() > MAX_NAME_BYTES {
            return YLB_E_ARGUMENT;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        let Some(mat) = usize::try_from(material)
            .ok()
            .and_then(|i| m.materials.get_mut(i))
        else {
            return YLB_E_ARGUMENT;
        };
        if mat.routes.iter().any(|r| r.channel == channel as u8) {
            return YLB_E_ARGUMENT;
        }
        mat.routes.push(ChannelRoute {
            channel: channel as u8,
            property: property.to_owned(),
        });
        0
    })
}

unsafe fn vec3s(ptr: *const f32, count: i32) -> Option<Vec<[f32; 3]>> {
    let flat = bytes(ptr, count.checked_mul(3)?)?;
    let v: Vec<[f32; 3]> = flat.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    v.iter().flatten().all(|x| x.is_finite()).then_some(v)
}

/// メッシュを足す（位置は根のローカルの空間。法線・UV0 は null なら無し）。返すのはメッシュの番号。
#[no_mangle]
pub unsafe extern "C" fn ylb_model_mesh(
    handle: u64,
    key: *const u8,
    key_len: i32,
    name: *const u8,
    name_len: i32,
    skinned: i32,
    positions: *const f32,
    normals: *const f32,
    uv0: *const f32,
    vertex_count: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let (Some(key), Some(name)) = (text(key, key_len), text(name, name_len)) else {
            return YLB_E_ARGUMENT;
        };
        if vertex_count < 0
            || vertex_count as usize > MAX_VERTICES
            || key.len() > MAX_NAME_BYTES
            || name.len() > MAX_NAME_BYTES
        {
            return YLB_E_ARGUMENT;
        }
        let Some(positions) = vec3s(positions, vertex_count) else {
            return YLB_E_ARGUMENT;
        };
        let normals = if normals.is_null() {
            Vec::new()
        } else {
            match vec3s(normals, vertex_count) {
                Some(v) => v,
                None => return YLB_E_ARGUMENT,
            }
        };
        let uv0 = if uv0.is_null() {
            Vec::new()
        } else {
            let Some(flat) = vertex_count.checked_mul(2).and_then(|n| bytes(uv0, n)) else {
                return YLB_E_ARGUMENT;
            };
            if !flat.iter().all(|x| x.is_finite()) {
                return YLB_E_ARGUMENT;
            }
            flat.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        if m.meshes.len() >= MAX_MESHES {
            return YLB_E_ARGUMENT;
        }
        m.meshes.push(MeshData {
            key: key.to_owned(),
            name: name.to_owned(),
            skinned: skinned != 0,
            positions,
            normals,
            uv0,
            submeshes: Vec::new(),
        });
        (m.meshes.len() - 1) as i32
    })
}

/// メッシュに三角形の組を足す（添字は 3 つずつ、頂点の数より小さい）。
#[no_mangle]
pub unsafe extern "C" fn ylb_model_submesh(
    handle: u64,
    mesh: i32,
    material: i32,
    indices: *const i32,
    index_count: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(idx) = bytes(indices, index_count) else {
            return YLB_E_ARGUMENT;
        };
        if idx.len() % 3 != 0 || idx.len() > MAX_INDICES {
            return YLB_E_ARGUMENT;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        let materials = m.materials.len();
        let Some(me) = usize::try_from(mesh).ok().and_then(|i| m.meshes.get_mut(i)) else {
            return YLB_E_ARGUMENT;
        };
        if material < 0 || material as usize >= materials || me.submeshes.len() >= MAX_SUBMESHES {
            return YLB_E_ARGUMENT;
        }
        let n = me.positions.len();
        if idx.iter().any(|&i| i < 0 || i as usize >= n) {
            return YLB_E_ARGUMENT;
        }
        me.submeshes.push(Submesh {
            material: material as u32,
            indices: idx.iter().map(|&i| i as u32).collect(),
        });
        0
    })
}

/// 組み立てたモデルを送る（積むだけ）。返すのはモデルの世代（1 から）。
#[no_mangle]
pub extern "C" fn ylb_model_send(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if s.status() != Status::Connected {
            return YLB_E_STATE;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(mut m) = b.model.take() else {
            return YLB_E_STATE;
        };
        let generation = b.sent_generation.wrapping_add(1).max(1);
        m.generation = generation;
        let vertices: Vec<usize> = m.meshes.iter().map(|x| x.positions.len()).collect();
        if !s.enqueue(&Message::Model(m)) {
            return YLB_E_STATE;
        }
        b.sent_generation = generation;
        b.sent_vertices = vertices;
        b.pose = None;
        generation as i32
    })
}

/// 送ったモデルを閉じる（スタンドアロンにもう見せないと知らせる）。
#[no_mangle]
pub extern "C" fn ylb_model_close(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let generation = s
            .builder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .sent_generation;
        if generation == 0 || !s.enqueue(&Message::ModelClosed { generation }) {
            return YLB_E_STATE;
        }
        0
    })
}

/// ポーズの組み立てを始める。
#[no_mangle]
pub extern "C" fn ylb_pose_begin(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        if b.sent_generation == 0 {
            return YLB_E_STATE;
        }
        b.pose = Some(Vec::new());
        0
    })
}

/// メッシュの新しい形を足す（頂点の数は送ったモデルのそのメッシュと同じ）。
#[no_mangle]
pub unsafe extern "C" fn ylb_pose_mesh(
    handle: u64,
    mesh: i32,
    positions: *const f32,
    normals: *const f32,
    vertex_count: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(expected) = usize::try_from(mesh)
            .ok()
            .and_then(|i| b.sent_vertices.get(i))
            .copied()
        else {
            return YLB_E_ARGUMENT;
        };
        if vertex_count < 0 || vertex_count as usize != expected {
            return YLB_E_ARGUMENT;
        }
        let Some(positions) = vec3s(positions, vertex_count) else {
            return YLB_E_ARGUMENT;
        };
        let normals = if normals.is_null() {
            Vec::new()
        } else {
            match vec3s(normals, vertex_count) {
                Some(v) => v,
                None => return YLB_E_ARGUMENT,
            }
        };
        let Some(pose) = b.pose.as_mut() else {
            return YLB_E_STATE;
        };
        pose.retain(|p| p.mesh != mesh as u32);
        pose.push(MeshPose {
            mesh: mesh as u32,
            positions,
            normals,
        });
        0
    })
}

/// 組み立てたポーズを送る（積むだけ。同じメッシュのまだ送っていない古いポーズは置き換える）。返すのは足したメッシュの数。
#[no_mangle]
pub extern "C" fn ylb_pose_send(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if s.status() != Status::Connected {
            return YLB_E_STATE;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(pose) = b.pose.take() else {
            return YLB_E_STATE;
        };
        let n = pose.len() as i32;
        if !s.enqueue_pose(b.sent_generation, pose) {
            return YLB_E_STATE;
        }
        n
    })
}

// ───────── テクスチャセットを受ける ─────────

/// テクスチャセット 1 つの情報。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct YlbSetInfo {
    pub set: u32,
    /// どのモデルの世代のマテリアルか（ylb_model_send の返した値）。
    pub generation: u32,
    /// モデルのマテリアルの番号。
    pub material: u32,
    pub width: u32,
    pub height: u32,
    pub tile_size: u32,
    /// 開けたチャンネルの印（1 << チャンネル）。
    pub channel_mask: u32,
    /// 知らせを受け直すたびに変わる番号（変われば C# はテクスチャを作り直す）。
    pub revision: u32,
}

/// テクスチャセットの数。
#[no_mangle]
pub extern "C" fn ylb_set_count(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || match session(handle) {
        Some(s) => s.state().sets.len() as i32,
        None => YLB_E_HANDLE,
    })
}

/// index 番目（セットの番号の順）のテクスチャセットの情報。
#[no_mangle]
pub unsafe extern "C" fn ylb_set_info(handle: u64, index: i32, info: *mut YlbSetInfo) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if info.is_null() {
            return YLB_E_ARGUMENT;
        }
        let st = s.state();
        let Some(set) = usize::try_from(index).ok().and_then(|i| st.sets.get(i)) else {
            return YLB_E_ARGUMENT;
        };
        *info = YlbSetInfo {
            set: set.info.set,
            generation: set.info.generation,
            material: set.info.material,
            width: set.info.width,
            height: set.info.height,
            tile_size: set.info.tile_size,
            channel_mask: set.channel_mask(),
            revision: set.revision,
        };
        0
    })
}

/// テクスチャセットの名前。要る長さを返す。
#[no_mangle]
pub unsafe extern "C" fn ylb_set_name(handle: u64, set: u32, buf: *mut u8, cap: i32) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let st = s.state();
        match st.sets.iter().find(|x| x.info.set == set) {
            Some(x) => put_text(&x.info.name, buf, cap),
            None => YLB_E_ARGUMENT,
        }
    })
}

/// チャンネルの汚れたタイルの数。
#[no_mangle]
pub extern "C" fn ylb_channel_dirty(handle: u64, set: u32, channel: i32) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let mut st = s.state();
        let Some(c) = st
            .set_mut(set)
            .and_then(|x| x.channel_mut(channel.clamp(0, 255) as u8))
        else {
            return YLB_E_ARGUMENT;
        };
        if c.image.is_none() {
            return YLB_E_SHM;
        }
        c.dirty_count as i32
    })
}

/// 写した結果。
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct YlbCopyResult {
    pub tiles: u32,
    /// 書いている途中だったので写さなかった（汚れたまま残した）タイル。
    pub torn: u32,
    /// まだ汚れているタイル。
    pub remaining: u32,
    /// 写した画素の範囲（max は含まない。写していなければ全部 0）。
    pub x_min: u32,
    pub y_min: u32,
    pub x_max: u32,
    pub y_max: u32,
    /// 一番新しい知らせの、スタンドアロンが書き終えた時刻と、ブリッジが受けた時刻（UNIX のマイクロ秒）。
    pub stamp_us: u64,
    pub received_us: u64,
}

/// 汚れたタイルを image（幅 × 高さ × 4 バイト、straight RGBA8、下の行から。Unity の Texture2D の RGBA32 の生の並び）へ写す。
/// strip が null でなければ、strip_tiles 個までのタイルを帯（幅 strip_tiles × タイル、高さ タイル、下の行から）にも並べ、
/// coords（2 × strip_tiles 個）にタイルの座標を書いて、そこで止める（残りは次に）。返すのは写したタイルの数。
#[no_mangle]
pub unsafe extern "C" fn ylb_copy_dirty(
    handle: u64,
    set: u32,
    channel: i32,
    image: *mut u8,
    image_len: u64,
    strip: *mut u8,
    strip_len: u64,
    strip_tiles: i32,
    coords: *mut u32,
    result: *mut YlbCopyResult,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if image.is_null() || image_len == 0 {
            return YLB_E_ARGUMENT;
        }
        let image = std::slice::from_raw_parts_mut(image, image_len as usize);
        let strip = if strip.is_null() {
            None
        } else {
            if strip_tiles <= 0 || coords.is_null() {
                return YLB_E_ARGUMENT;
            }
            Some(Strip {
                pixels: std::slice::from_raw_parts_mut(strip, strip_len as usize),
                capacity: strip_tiles as usize,
                coords: std::slice::from_raw_parts_mut(coords, strip_tiles as usize * 2),
            })
        };
        let mut st = s.state();
        let Some(c) = st
            .set_mut(set)
            .and_then(|x| x.channel_mut(channel.clamp(0, 255) as u8))
        else {
            return YLB_E_ARGUMENT;
        };
        match copy_dirty(c, image, strip) {
            Ok(o) => {
                if !result.is_null() {
                    *result = YlbCopyResult {
                        tiles: o.tiles,
                        torn: o.torn,
                        remaining: o.remaining,
                        x_min: o.bbox[0],
                        y_min: o.bbox[1],
                        x_max: o.bbox[2],
                        y_max: o.bbox[3],
                        stamp_us: c.stamp_us,
                        received_us: c.received_us,
                    };
                }
                o.tiles as i32
            }
            Err(ShmError::OutOfRange(_)) => YLB_E_ARGUMENT,
            Err(_) => YLB_E_SHM,
        }
    })
}

/// UNIX 時刻のマイクロ秒（スタンドアロン・ブリッジと同じ時計で遅れを測る）。
#[no_mangle]
pub extern "C" fn ylb_now_us() -> u64 {
    yolu_protocol::host::now_us()
}

// ───────── 自己診断のスタンドアロン（同じプロセスの中。Unity の試験と、スタンドアロン無しでの確かめ用） ─────────

/// 試しの模様を返すスタンドアロンを、このプロセスの中で待ち受けさせる。モデルが来ると、マテリアルごとに size × size のセット
/// （Color と、流し込み先のあるチャンネル）を作り、ylb_test_server_pattern の模様を全部のタイルに書いて知らせる。返すのは番号（0 は失敗）。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_start(
    name: *const u8,
    name_len: i32,
    size: i32,
    tile_size: i32,
) -> u64 {
    guard(0, || {
        let Some(name) = text(name, name_len) else {
            return 0;
        };
        if !(1..=MAX_TEXTURE_SIZE as i32).contains(&size)
            || !yolu_protocol::shm::valid_tile_size(tile_size.max(0) as u32)
        {
            return 0;
        }
        match TestServer::start(name, size as u32, tile_size as u32) {
            Ok(server) => {
                let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
                SERVERS
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(id, server);
                id
            }
            Err(_) => 0,
        }
    })
}

/// 自己診断のスタンドアロンを止める（つないでいる側を先に切る。Windows では相手が切るまで読むスレッドが残る）。
#[no_mangle]
pub extern "C" fn ylb_test_server_stop(server: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        match SERVERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&server)
        {
            Some(s) => {
                s.stop();
                0
            }
            None => YLB_E_HANDLE,
        }
    })
}

/// 模様の色（RGBA を r | g << 8 | b << 16 | a << 24 に詰めたもの）: マテリアルの番号ごとの色の市松（奇数のタイルは半分の明るさ）。
#[no_mangle]
pub extern "C" fn ylb_test_server_pattern(material: u32, tile_x: u32, tile_y: u32) -> u32 {
    let [r, g, b, a] = crate::testserver::pattern(material, tile_x, tile_y);
    u32::from_le_bytes([r, g, b, a])
}

/// 自己診断のスタンドアロンで、material の番号のセットの channel の、タイル [x0, x1) × [y0, y1) を色 rgba（ylb_test_server_pattern
/// と同じ詰め方）で塗り、変わったタイルを知らせる。返すのは塗ったタイルの数。
#[no_mangle]
pub extern "C" fn ylb_test_server_paint(
    server: u64,
    material: u32,
    channel: i32,
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    rgba: u32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        if !(0..channel::COUNT as i32).contains(&channel) {
            return YLB_E_ARGUMENT;
        }
        s.paint(material, channel as u8, x0, y0, x1, y1, rgba.to_le_bytes())
    })
}

/// 自己診断のスタンドアロンが受けたものの数。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_stats(server: u64, stats: *mut YlbTestServerStats) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        if stats.is_null() {
            return YLB_E_ARGUMENT;
        }
        *stats = s.stats();
        0
    })
}
