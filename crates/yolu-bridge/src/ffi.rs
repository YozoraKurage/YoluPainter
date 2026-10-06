//! C の関数（Unity の C# から P/Invoke で呼ぶ。C# の宣言は csbindgen がこのファイルから作る）。
//!
//! 決まり:
//! - Unity の主スレッドから呼ぶ前提で、どれも待たない（つなぐ・送る・受けるは裏のスレッド。ここは錠を取って状態を読み書きするだけ）。
//!   画素の写し（ylb_copy_dirty）だけは写す量に比例して時間がかかる。
//! - 返す値: 0 以上は成功、負は失敗（YLB_E_*）。文字列は UTF-8 のバイトと長さ。受け取る文字列の領域が足りなければ、要る長さを返して
//!   入るだけ（文字の途中で切らない）写す。
//! - Unity は一度読んだネイティブの DLL を手放さない。ドメインの読み直しの後も前のつながりが残るので、C# は最初に ylb_disconnect_all を呼ぶ。
//!   この口は小さく保ち、変えるときは関数を足す（今の関数の意味を変えるか、C# が新しい関数に頼るなら ylb_abi_version を上げ、C# は合わない版を使わない）。
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
use crate::session::{EnqueueError, Session, Status};
use crate::testserver::{
    TestServer, YlbTestServerOriginal, YlbTestServerStats, YlbTestServerTexture,
};

/// この口の版。関数の意味・引数・構造体を変えたら、または C# が新しく足した関数・欄に頼るようになったら上げる（Unity は読んだ DLL を
/// 手放さないので、古い DLL のまま新しい C# が動くと、足りない関数で空回りして理由も出ない。版が違えば C# は使わず、理由を出す）。
/// 2: マテリアルの更新（ylb_materials_*）・全面の写し直し（ylb_channel_mark_all_dirty）・帯だけの写し（ylb_copy_dirty の image が null）・
/// 自己診断のサーバーの鍵の差し替えと統計の欄の追加。
/// 3: 互いの版と機能の印（ylb_connect_with・ylb_common_features・ylb_peer_app_version・ylb_link_report・ylb_test_server_configure）。
/// 4: マテリアルの値（ylb_values_*・ylb_texture_send）、自己診断のサーバーの値の引き出し（ylb_test_server_value・_slot・_texture）と
/// 統計の欄の追加。
/// 5: 元の絵（ylb_original_send・ylb_pending_bytes）、自己診断のサーバーの元の絵の引き出し（ylb_test_server_original）と統計の欄の追加。
/// 6: スタンドアロンからの頼み（ylb_next_request）、元の絵の印と「持っている絵を使う」様子（ylb_original_send に stamp を足し、state に 4）、
/// 巨大なモデルの断り（YLB_E_TOO_LARGE）、自己診断のサーバーの頼みの送り出し（ylb_test_server_request）と元の絵の印・統計の欄の追加。
/// 7: 送りの列の上限と「混んでいる」（YLB_E_BUSY。送る関数が返す。送り直せる）、いま積める大きさ（ylb_send_room）、ylb_pending_bytes が書いている途中の枠も
/// 数えること、試験用の上限の変更（ylb_test_set_outbox_limit）と自己診断のサーバーの読みの停止（ylb_test_server_pause_reading）。
pub const ABI_VERSION: u32 = 7;

/// このブリッジが挨拶で出す機能の印（`yolu_protocol::feature`）。印を立てる機能を足すときは、ここに `feature` のビットを足す。
/// 元のテクスチャ（ORIGINAL_TEXTURES）: スタンドアロンが新しく作ったテクスチャセットの一番下に入れる、元の絵を送る。
/// マテリアルの頼み（MATERIAL_REQUEST）: スタンドアロンの頼み（`ylb_next_request`）に答える。印が双方にあるとき、C# は元の絵を自分から押し出さない。
pub const BRIDGE_FEATURES: u64 = yolu_protocol::feature::MATERIAL_VALUES
    | yolu_protocol::feature::ORIGINAL_TEXTURES
    | yolu_protocol::feature::MATERIAL_REQUEST;

pub const YLB_E_HANDLE: i32 = -1;
pub const YLB_E_ARGUMENT: i32 = -2;
pub const YLB_E_STATE: i32 = -3;
pub const YLB_E_PANIC: i32 = -5;
pub const YLB_E_SHM: i32 = -6;
/// 命令が枠の上限（512 MiB）を超えるので送れない（巨大なモデル）。
pub const YLB_E_TOO_LARGE: i32 = -7;
/// 送りの列が混んでいる（相手が読むのを待っていて、積んである量が上限 `MAX_OUTBOX_BYTES` に近い）ので積まなかった。何も変えていないので、少し後に
/// 同じ物を送り直せる（`ylb_model_send`・`ylb_materials_send`・`ylb_values_send` は組み立てを残すので、組み立て直さず同じ関数をもう一度呼べる）。
/// `ylb_send_room` で、積める大きさを先に確かめられる。
pub const YLB_E_BUSY: i32 = -8;

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

/// 命令を積んだ結果を C の返す値にする: 積めたら `queued`（関数ごとの成功の値）、印が無い・閉じていれば YLB_E_STATE、枠の上限を超えれば
/// YLB_E_TOO_LARGE、列が混んでいれば YLB_E_BUSY。
fn queued_code(session: &Session, message: &Message, queued: i32) -> i32 {
    match session.try_enqueue(message) {
        Ok(true) => queued,
        Ok(false) => YLB_E_STATE,
        Err(EnqueueError::TooLarge(_)) => YLB_E_TOO_LARGE,
        Err(EnqueueError::Busy) => YLB_E_BUSY,
    }
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
    ylb_connect_with(name, name_len, agent, agent_len, std::ptr::null(), 0)
}

/// `ylb_connect`（自分のアプリの版を挨拶で名乗る。Unity のパッケージの版の文字列「0.3.0」など。読めない・空なら名乗らない）。
#[no_mangle]
pub unsafe extern "C" fn ylb_connect_with(
    name: *const u8,
    name_len: i32,
    agent: *const u8,
    agent_len: i32,
    app_version: *const u8,
    app_version_len: i32,
) -> u64 {
    guard(0, || {
        let Some(name) = text(name, name_len) else {
            return 0;
        };
        let Some(agent) = text(agent, agent_len) else {
            return 0;
        };
        let Some(version) = text(app_version, app_version_len) else {
            return 0;
        };
        if !yolu_protocol::link::valid_link_name(name) {
            return 0;
        }
        let identity = Identity::unity(&format!("{agent} (yolu-bridge abi {ABI_VERSION})"))
            .with_version(AppVersion::parse(version))
            .with_features(BRIDGE_FEATURES);
        let id = NEXT_HANDLE.fetch_add(1, Ordering::Relaxed);
        let s = Session::start(id, name.to_owned(), identity);
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

/// このつながりで使える機能の印（双方が出した印の共通部分。つながるまでは 0）。印の要る新しい命令は、ここに立っているときだけ送る。
#[no_mangle]
pub extern "C" fn ylb_common_features(handle: u64) -> u64 {
    guard(0, || session(handle).map_or(0, |s| s.common_features()))
}

/// 相手（スタンドアロン）のアプリの版（`major << 32 | minor << 16 | patch`）。つながっていない・版を名乗らない古い相手は `u64::MAX`。
#[no_mangle]
pub extern "C" fn ylb_peer_app_version(handle: u64) -> u64 {
    guard(AppVersion::NONE_PACKED, || {
        session(handle)
            .and_then(|s| s.state().link.as_ref().map(|l| l.peer.app_version()))
            .map_or(AppVersion::NONE_PACKED, compat::pack_version)
    })
}

/// 版のずれと機能の印の様子（`ylb_link_report`）。版は `major << 32 | minor << 16 | patch`、不明は `u64::MAX`。
#[repr(C)]
#[derive(Clone, Copy)]
pub struct YlbLinkReport {
    /// 相手のアプリの版。
    pub peer_version: u64,
    /// 相手が自分に求める版（相手が宣言していなければ不明）。
    pub peer_min_peer: u64,
    /// 相手を上げるべきなら、求める版（`0` は版の指定なし）。上げなくてよければ不明。版を名乗らない古い相手は上げるべき。
    pub update_peer: u64,
    /// 自分（Unity のパッケージ）を上げるべきなら、求める版。上げなくてよければ不明。
    pub update_self: u64,
    /// 自分が出した機能の印・相手が出した印・共通部分（使える機能）。
    pub own_features: u64,
    pub peer_features: u64,
    pub common_features: u64,
    /// 自分にあって相手に無い機能（相手を上げれば使える）・相手にあって自分に無い機能（自分を上げれば使える）。
    pub missing_on_peer: u64,
    pub missing_here: u64,
    /// 断られたとき（state 2）、上げるべき製品: 1 = Unity のパッケージ、2 = スタンドアロン。それ以外は 0。
    pub refused_update: i32,
    /// 断られたときの、上げるべき製品の求める版（求める版が決まっていなければ不明）。
    pub refused_to: u64,
    /// 断られたときの、Unity 側・スタンドアロンの読めるプロトコルの版の範囲。
    pub unity_min_protocol: u32,
    pub unity_max_protocol: u32,
    pub standalone_min_protocol: u32,
    pub standalone_max_protocol: u32,
    /// 0 = まだ無い（つないでいる・つなげなかった）、1 = つながった（上の欄を決めた）、2 = プロトコルの版の範囲が合わず断られた。
    pub state: i32,
    /// つながったときの、決まったプロトコルの版。
    pub protocol: u32,
}

/// 版のずれと機能の印の様子を取り出す。返すのは `state`（0 = まだ無い、1 = つながった、2 = 版の範囲が合わず断られた）、負は失敗。
#[no_mangle]
pub unsafe extern "C" fn ylb_link_report(handle: u64, report: *mut YlbLinkReport) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if report.is_null() {
            return YLB_E_ARGUMENT;
        }
        let none = AppVersion::NONE_PACKED;
        let mut r = YlbLinkReport {
            peer_version: none,
            peer_min_peer: none,
            update_peer: none,
            update_self: none,
            own_features: 0,
            peer_features: 0,
            common_features: 0,
            missing_on_peer: 0,
            missing_here: 0,
            refused_update: 0,
            refused_to: none,
            unity_min_protocol: 0,
            unity_max_protocol: 0,
            standalone_min_protocol: 0,
            standalone_max_protocol: 0,
            state: 0,
            protocol: 0,
        };
        {
            let st = s.state();
            if let Some(info) = &st.link {
                let skew = info.skew();
                r.state = 1;
                r.protocol = info.protocol as u32;
                r.peer_version = compat::pack_version(info.peer.app_version());
                r.peer_min_peer = compat::pack_version(info.peer.versions.map(|v| v.min_peer));
                r.update_peer = compat::pack_version(skew.update_peer);
                r.update_self = compat::pack_version(skew.update_self);
                r.own_features = info.own.features;
                r.peer_features = info.peer.features;
                r.common_features = info.common_features();
                r.missing_on_peer = skew.missing_on_peer;
                r.missing_here = skew.missing_here;
            } else if let Some(refusal) = &st.refusal {
                r.state = 2;
                r.refused_update = match refusal.update {
                    Product::Unity => 1,
                    Product::Standalone => 2,
                };
                r.refused_to = compat::pack_version(refusal.to);
                r.unity_min_protocol = refusal.unity_range.0 as u32;
                r.unity_max_protocol = refusal.unity_range.1 as u32;
                r.standalone_min_protocol = refusal.standalone_range.0 as u32;
                r.standalone_max_protocol = refusal.standalone_range.1 as u32;
            }
        }
        *report = r;
        r.state
    })
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

/// マテリアルの組を足す（モデルとマテリアルの更新で同じ）。返すのはマテリアルの番号か、負の失敗。
unsafe fn push_material(
    materials: &mut Vec<MaterialInfo>,
    unassigned: i32,
    name: (*const u8, i32),
    guid: (*const u8, i32),
    file_id: i64,
    shader: (*const u8, i32),
) -> i32 {
    let (Some(name), Some(guid), Some(shader)) = (
        text(name.0, name.1),
        text(guid.0, guid.1),
        text(shader.0, shader.1),
    ) else {
        return YLB_E_ARGUMENT;
    };
    if name.len() > MAX_NAME_BYTES
        || shader.len() > MAX_NAME_BYTES
        || (!guid.is_empty() && !is_asset_guid(guid))
        || materials.len() >= MAX_MATERIALS
    {
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
    materials.push(MaterialInfo {
        key,
        shader: shader.to_owned(),
        textures: Vec::new(),
        routes: Vec::new(),
    });
    (materials.len() - 1) as i32
}

/// マテリアルにテクスチャのプロパティを足す。
unsafe fn push_texture(
    materials: &mut [MaterialInfo],
    material: i32,
    name: (*const u8, i32),
    width: u32,
    height: u32,
) -> i32 {
    let Some(name) = text(name.0, name.1) else {
        return YLB_E_ARGUMENT;
    };
    let Some(mat) = usize::try_from(material)
        .ok()
        .and_then(|i| materials.get_mut(i))
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
}

/// マテリアルに見せられるチャンネルと流し込み先を足す。
unsafe fn push_route(
    materials: &mut [MaterialInfo],
    material: i32,
    channel: i32,
    property: (*const u8, i32),
) -> i32 {
    let Some(property) = text(property.0, property.1) else {
        return YLB_E_ARGUMENT;
    };
    if !(0..channel::COUNT as i32).contains(&channel) || property.len() > MAX_NAME_BYTES {
        return YLB_E_ARGUMENT;
    }
    let Some(mat) = usize::try_from(material)
        .ok()
        .and_then(|i| materials.get_mut(i))
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
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        push_material(
            &mut m.materials,
            unassigned,
            (name, name_len),
            (guid, guid_len),
            file_id,
            (shader, shader_len),
        )
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
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        push_texture(&mut m.materials, material, (name, name_len), width, height)
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
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.model.as_mut() else {
            return YLB_E_STATE;
        };
        push_route(
            &mut m.materials,
            material,
            channel,
            (property, property_len),
        )
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

/// 組み立てたモデルを送る（積むだけ）。返すのはモデルの世代（1 から）。送りの列が混んでいれば YLB_E_BUSY（組み立てを残すので、少し後にもう一度呼べる。断るときはモデルを枠にしないので、呼び直しは安い）。
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
        // 枠の上限を超えるモデルは、書き出す前に断る（枠にしようとして領域を取り、相手の読み手につながりを閉じさせない）
        if m.payload_len() > s.payload_limit() as u64 {
            return YLB_E_TOO_LARGE;
        }
        let generation = b.sent_generation.wrapping_add(1).max(1);
        m.generation = generation;
        let vertices: Vec<usize> = m.meshes.iter().map(|x| x.positions.len()).collect();
        let material_count = m.materials.len();
        let message = Message::Model(m);
        match s.try_enqueue(&message) {
            Ok(true) => {}
            Ok(false) => return YLB_E_STATE,
            Err(EnqueueError::TooLarge(_)) => return YLB_E_TOO_LARGE,
            // 混んでいる: 組み立てたモデルを残す（読み直さずに、少し後に ylb_model_send だけをもう一度呼べる）
            Err(EnqueueError::Busy) => {
                if let Message::Model(m) = message {
                    b.model = Some(m);
                }
                return YLB_E_BUSY;
            }
        }
        b.sent_generation = generation;
        b.sent_vertices = vertices;
        b.sent_materials = material_count;
        b.pose = None;
        b.materials = None;
        b.values = None;
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

// ───────── マテリアルの更新を送る（モデルを送り直さずに、シェーダー・テクスチャのプロパティ・流し込み先だけを変える） ─────────

/// マテリアルの更新の組み立てを始める（前の組み立ては捨てる）。送ったモデルがあるときだけ。足すマテリアルの数と並びは、送ったモデルと同じにする。
#[no_mangle]
pub extern "C" fn ylb_materials_begin(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        if b.sent_generation == 0 {
            return YLB_E_STATE;
        }
        b.materials = Some(Vec::new());
        0
    })
}

/// 更新のマテリアルの組を足す（`ylb_model_material` と同じ引数）。返すのはマテリアルの番号。
#[no_mangle]
pub unsafe extern "C" fn ylb_materials_material(
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
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.materials.as_mut() else {
            return YLB_E_STATE;
        };
        push_material(
            m,
            unassigned,
            (name, name_len),
            (guid, guid_len),
            file_id,
            (shader, shader_len),
        )
    })
}

/// 更新のマテリアルにテクスチャのプロパティを足す。
#[no_mangle]
pub unsafe extern "C" fn ylb_materials_texture(
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
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.materials.as_mut() else {
            return YLB_E_STATE;
        };
        push_texture(m, material, (name, name_len), width, height)
    })
}

/// 更新のマテリアルに、見せられるチャンネルと流し込み先を足す。
#[no_mangle]
pub unsafe extern "C" fn ylb_materials_route(
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
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(m) = b.materials.as_mut() else {
            return YLB_E_STATE;
        };
        push_route(m, material, channel, (property, property_len))
    })
}

/// 組み立てたマテリアルの更新を送る（積むだけ）。マテリアルの数は送ったモデルと同じでなければならない（違えば YLB_E_ARGUMENT）。返すのはマテリアルの数。
/// 送りの列が混んでいれば YLB_E_BUSY（組み立てを残すので、少し後にもう一度呼べる）。同じ世代の送っていない古い更新は置き換わる。
#[no_mangle]
pub extern "C" fn ylb_materials_send(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if s.status() != Status::Connected {
            return YLB_E_STATE;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(materials) = b.materials.take() else {
            return YLB_E_STATE;
        };
        if materials.len() != b.sent_materials {
            return YLB_E_ARGUMENT;
        }
        let n = materials.len() as i32;
        let generation = b.sent_generation;
        let message = Message::Materials(MaterialsUpdate {
            generation,
            materials,
        });
        let code = queued_code(&s, &message, n);
        if code == YLB_E_BUSY {
            // 混んでいる: 組み立てを残す（少し後にもう一度呼べる）
            if let Message::Materials(u) = message {
                b.materials = Some(u.materials);
            }
        }
        code
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

// ───────── マテリアルの値を送る（機能の印 MATERIAL_VALUES がスタンドアロンにもあるときだけ。lilToon のプロパティの値と描いていないスロットの絵） ─────────

/// マテリアルの値の組み立てを始める（前の組み立ては捨てる）。送ったモデルがあるときだけ。`material` は送ったモデルのマテリアルの番号、
/// `kind` は 0 = 値なし（前に送った値を捨てさせる）・1 = lilToon、`source` は何の対応と確かめたかの文（人に見せるだけ）。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_begin(
    handle: u64,
    material: i32,
    kind: i32,
    shader: *const u8,
    shader_len: i32,
    source: *const u8,
    source_len: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let (Some(shader), Some(source)) = (text(shader, shader_len), text(source, source_len))
        else {
            return YLB_E_ARGUMENT;
        };
        let kind = match kind {
            0 => ValuesKind::None,
            1 => ValuesKind::LilToon,
            _ => return YLB_E_ARGUMENT,
        };
        if shader.len() > MAX_NAME_BYTES || source.len() > MAX_NAME_BYTES {
            return YLB_E_ARGUMENT;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        if b.sent_generation == 0 {
            return YLB_E_STATE;
        }
        if material < 0 || material as usize >= b.sent_materials {
            return YLB_E_ARGUMENT;
        }
        b.values = Some(MaterialValues {
            generation: b.sent_generation,
            material: material as u32,
            kind,
            shader: shader.to_owned(),
            source: source.to_owned(),
            properties: Vec::new(),
            keywords: Vec::new(),
            slots: Vec::new(),
        });
        0
    })
}

/// 値の名前（1〜MAX_VALUE_NAME_BYTES バイトの UTF-8、制御文字なし）。
unsafe fn value_name<'a>(name: *const u8, len: i32) -> Option<&'a str> {
    text(name, len).filter(|n| {
        !n.is_empty() && n.len() <= MAX_VALUE_NAME_BYTES && !n.chars().any(char::is_control)
    })
}

/// 組み立て中の値にプロパティを足す（同じ名前は置き換える）。
unsafe fn push_value(handle: u64, name: (*const u8, i32), value: PropertyValue) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = value_name(name.0, name.1) else {
            return YLB_E_ARGUMENT;
        };
        let finite = match value {
            PropertyValue::Float(x) => x.is_finite(),
            PropertyValue::Int(_) => true,
            PropertyValue::Color(c) | PropertyValue::Vector(c) => c.iter().all(|x| x.is_finite()),
        };
        if !finite {
            return YLB_E_ARGUMENT;
        }
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = b.values.as_mut() else {
            return YLB_E_STATE;
        };
        if let Some(p) = v.properties.iter_mut().find(|p| p.name == name) {
            p.value = value;
            return 0;
        }
        if v.properties.len() >= MAX_VALUE_PROPERTIES {
            return YLB_E_ARGUMENT;
        }
        v.properties.push(PropertyEntry {
            name: name.to_owned(),
            value,
        });
        0
    })
}

/// Float・Range の値を足す（有限の数だけ）。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_float(
    handle: u64,
    name: *const u8,
    name_len: i32,
    value: f32,
) -> i32 {
    push_value(handle, (name, name_len), PropertyValue::Float(value))
}

/// Integer の値を足す。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_int(
    handle: u64,
    name: *const u8,
    name_len: i32,
    value: i32,
) -> i32 {
    push_value(handle, (name, name_len), PropertyValue::Int(value))
}

/// 色の値を足す（マテリアルに入っているままの値。`[HDR]` でない色はガンマの空間）。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_color(
    handle: u64,
    name: *const u8,
    name_len: i32,
    r: f32,
    g: f32,
    b: f32,
    a: f32,
) -> i32 {
    push_value(handle, (name, name_len), PropertyValue::Color([r, g, b, a]))
}

/// ベクトルの値を足す（テクスチャのタイリング・オフセットは `<名前>_ST` の名前で）。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_vector(
    handle: u64,
    name: *const u8,
    name_len: i32,
    x: f32,
    y: f32,
    z: f32,
    w: f32,
) -> i32 {
    push_value(
        handle,
        (name, name_len),
        PropertyValue::Vector([x, y, z, w]),
    )
}

/// 有効なキーワードを足す（重ねて足したものは 1 つ）。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_keyword(handle: u64, name: *const u8, name_len: i32) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = value_name(name, name_len).filter(|n| !n.contains(' ')) else {
            return YLB_E_ARGUMENT;
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = b.values.as_mut() else {
            return YLB_E_STATE;
        };
        if v.keywords.iter().any(|k| k == name) {
            return 0;
        }
        if v.keywords.len() >= MAX_VALUE_KEYWORDS {
            return YLB_E_ARGUMENT;
        }
        v.keywords.push(name.to_owned());
        0
    })
}

/// 描いていないスロットの様子を足す。`state`: 0 入っていない・1 絵を送る（この値を送った後に ylb_texture_send）・2 前に送った絵と同じ・
/// 3 予算を超えて送らない・4 読めない。`width`・`height` は元のテクスチャの大きさ。
#[no_mangle]
pub unsafe extern "C" fn ylb_values_slot(
    handle: u64,
    name: *const u8,
    name_len: i32,
    state: i32,
    width: u32,
    height: u32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = value_name(name, name_len) else {
            return YLB_E_ARGUMENT;
        };
        let state = match state {
            0 => SlotState::Empty,
            1 => SlotState::Follows,
            2 => SlotState::Unchanged,
            3 => SlotState::OverBudget,
            4 => SlotState::Unreadable,
            _ => return YLB_E_ARGUMENT,
        };
        let mut b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
        let Some(v) = b.values.as_mut() else {
            return YLB_E_STATE;
        };
        if v.slots.iter().any(|x| x.name == name) || v.slots.len() >= MAX_VALUE_SLOTS {
            return YLB_E_ARGUMENT;
        }
        v.slots.push(SlotTexture {
            name: name.to_owned(),
            state,
            width,
            height,
        });
        0
    })
}

/// 組み立てた値を送る（積むだけ）。返すのは 1 = 積んだ、0 = スタンドアロンに印（MATERIAL_VALUES）が無いので送らない（組み立ては捨てる）。
/// 送りの列が混んでいれば YLB_E_BUSY（組み立てを残すので、少し後にもう一度呼べる）。同じマテリアルの送っていない古い値と絵は置き換わる
/// （新しい値が「前と同じ」と言う絵が列の中にあるときを除く）。
#[no_mangle]
pub extern "C" fn ylb_values_send(handle: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if s.status() != Status::Connected {
            return YLB_E_STATE;
        }
        let Some(values) = s
            .builder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values
            .take()
        else {
            return YLB_E_STATE;
        };
        let message = Message::MaterialValues(values);
        if !compat::accepts(s.common_features(), &message) {
            return 0;
        }
        let code = queued_code(&s, &message, 1);
        if code == YLB_E_BUSY {
            // 混んでいる: 組み立てを残す（少し後にもう一度呼べる）
            if let Message::MaterialValues(v) = message {
                s.builder.lock().unwrap_or_else(|e| e.into_inner()).values = Some(v);
            }
        }
        code
    })
}

/// 描いていないスロットの絵を送る（積むだけ。直前の値で状態 1 と言ったスロット）。`pixels` は RGBA8（straight）で行は下から、
/// `pixel_len` は幅 × 高さ × 4。辺は MAX_SLOT_TEXTURE_SIZE まで（送る側が縮める）。`srgb` が 0 でなければ Unity はこの絵を sRGB として
/// 読む。返すのは 1 = 積んだ、0 = スタンドアロンに印が無いので送らない。
/// 送りの列が混んでいれば YLB_E_BUSY（呼び手が画素を持っていて、少し後に送り直す。同じ世代・マテリアル・スロットの送っていない古い絵は置き換わる）。
#[no_mangle]
pub unsafe extern "C" fn ylb_texture_send(
    handle: u64,
    material: i32,
    slot: *const u8,
    slot_len: i32,
    width: u32,
    height: u32,
    srgb: i32,
    pixels: *const u8,
    pixel_len: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(slot) = value_name(slot, slot_len) else {
            return YLB_E_ARGUMENT;
        };
        if width == 0
            || height == 0
            || width > MAX_SLOT_TEXTURE_SIZE
            || height > MAX_SLOT_TEXTURE_SIZE
            || pixel_len as i64 != width as i64 * height as i64 * 4
        {
            return YLB_E_ARGUMENT;
        }
        let Some(pixels) = bytes(pixels, pixel_len) else {
            return YLB_E_ARGUMENT;
        };
        if s.status() != Status::Connected {
            return YLB_E_STATE;
        }
        let (generation, materials) = {
            let b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
            (b.sent_generation, b.sent_materials)
        };
        if generation == 0 {
            return YLB_E_STATE;
        }
        if material < 0 || material as usize >= materials {
            return YLB_E_ARGUMENT;
        }
        if !compat::satisfies(
            s.common_features(),
            Kind::MaterialTexture.required_feature(),
        ) {
            return 0;
        }
        let message = Message::MaterialTexture(MaterialTexture {
            generation,
            material: material as u32,
            slot: slot.to_owned(),
            width,
            height,
            srgb: srgb != 0,
            pixels: pixels.to_vec(),
        });
        queued_code(&s, &message, 1)
    })
}

/// 元の絵を送る（積むだけ）。Color の流し込み先のスロットの元のテクスチャ 1 つ（直前のモデルのマテリアルで絵が入っているもの）。
/// `state` は 0 = 絵が付く・1 = 読めない・2 = 辺が上限を超える・3 = 全部の絵の予算を超える（1〜3 は画素なし。`width`・`height` は元の
/// テクスチャの大きさ）、`read` は 0 = 原本のファイル・1 = 取り込んだ絵の CPU の値・2 = GPU を通して、`flags` の bit0 は圧縮された
/// テクスチャから読んだ。`pixels` は RGBA8（straight）で行は下から、`pixel_len` は幅 × 高さ × 4（辺は MAX_ORIGINAL_SIZE まで）。
/// `srgb` が 0 でなければ Unity はこの絵を sRGB として読む（ガンマの色空間のプロジェクトは真で送る）。
/// `stamp` はこの絵の印（Unity が決める 64 ビット。0 は印なし）。`state` が 4（スタンドアロンが頼みで持つと言った絵と印が同じ。画素なし）のときは
/// 0 以外が要る（`width`・`height` は Unity のテクスチャの大きさ）。
/// 返すのは 1 = 積んだ、0 = スタンドアロンに印が無いので送らない。送りの列が混んでいれば YLB_E_BUSY（呼び手が画素を持っていて、少し後に送り直す。
/// 画素の付いた元の絵は、同じ世代・マテリアル・スロットの送っていない古い元の絵を置き換える）。大きな絵は、読む前に `ylb_send_room` で入るかを確かめる。
#[no_mangle]
pub unsafe extern "C" fn ylb_original_send(
    handle: u64,
    material: i32,
    slot: *const u8,
    slot_len: i32,
    state: i32,
    read: i32,
    flags: i32,
    width: u32,
    height: u32,
    srgb: i32,
    stamp: u64,
    pixels: *const u8,
    pixel_len: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        let Some(slot) = value_name(slot, slot_len) else {
            return YLB_E_ARGUMENT;
        };
        if !(0..=4).contains(&state) || !(0..=2).contains(&read) {
            return YLB_E_ARGUMENT;
        }
        if state == OriginalState::Cached as i32 && stamp == 0 {
            return YLB_E_ARGUMENT;
        }
        let state = OriginalState::from_u8(state as u8);
        let size_ok = width > 0
            && height > 0
            && width <= MAX_ORIGINAL_SIZE
            && height <= MAX_ORIGINAL_SIZE
            && pixel_len as i64 == width as i64 * height as i64 * 4;
        let ok = match state {
            OriginalState::Image => size_ok,
            _ => pixel_len == 0,
        };
        if !ok {
            return YLB_E_ARGUMENT;
        }
        let Some(pixels) = bytes(pixels, pixel_len) else {
            return YLB_E_ARGUMENT;
        };
        if s.status() != Status::Connected {
            return YLB_E_STATE;
        }
        let (generation, materials) = {
            let b = s.builder.lock().unwrap_or_else(|e| e.into_inner());
            (b.sent_generation, b.sent_materials)
        };
        if generation == 0 {
            return YLB_E_STATE;
        }
        if material < 0 || material as usize >= materials {
            return YLB_E_ARGUMENT;
        }
        if !compat::satisfies(
            s.common_features(),
            Kind::MaterialOriginal.required_feature(),
        ) {
            return 0;
        }
        let message = Message::MaterialOriginal(MaterialOriginal {
            generation,
            material: material as u32,
            slot: slot.to_owned(),
            state,
            read: OriginalRead::from_u8(read as u8),
            compressed: flags & ORIGINAL_COMPRESSED as i32 != 0,
            width,
            height,
            srgb: srgb != 0,
            pixels: pixels.to_vec(),
            stamp,
        });
        queued_code(&s, &message, 1)
    })
}

/// 試験用: 1 つの命令の中身の上限（バイト）を狭める（`MAX_PAYLOAD` より大きくはならない）。巨大なモデルを作らずに、上限を超える
/// モデルの断り（YLB_E_TOO_LARGE）を確かめる。
#[no_mangle]
pub extern "C" fn ylb_test_set_payload_limit(handle: u64, bytes: u64) -> i32 {
    guard(YLB_E_PANIC, || match session(handle) {
        Some(s) => {
            s.set_payload_limit(usize::try_from(bytes).unwrap_or(usize::MAX));
            0
        }
        None => YLB_E_HANDLE,
    })
}

/// 試験用: 送りの列の上限（バイト）を決める（既定は 256 MiB）。小さい量で「混んでいる」（YLB_E_BUSY）を確かめる。
#[no_mangle]
pub extern "C" fn ylb_test_set_outbox_limit(handle: u64, bytes: u64) -> i32 {
    guard(YLB_E_PANIC, || match session(handle) {
        Some(s) => {
            s.set_outbox_limit(usize::try_from(bytes).unwrap_or(usize::MAX));
            0
        }
        None => YLB_E_HANDLE,
    })
}

/// スタンドアロンからの頼み 1 つ（`ylb_next_request`）。
#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct YlbRequest {
    /// 頼みの世代（`ylb_model_send` の返した値。今のモデルの世代と違う頼みは、古いモデルのもの）。
    pub generation: u32,
    /// モデルのマテリアルの番号。
    pub material: u32,
    /// 頼むもの（bit0 値・bit1 元の絵。知らない bit は無視する）。
    pub wants: u32,
    /// スロットの名前の長さ（バイト。入りきらなければ要る長さ。元の絵でなければ 0）。
    pub slot_len: i32,
    /// スタンドアロンが手元に持つ元の絵の印（0 は持たない）。
    pub have: u64,
}

/// スタンドアロンからの頼みを 1 つ取り出す（待たない）。1 = 取り出した（`request` と、スロットの名前を `slot` へ UTF-8 で。入りきらなければ
/// 文字の途中で切って、`slot_len` に要る長さ）、0 = 頼みは無い、負は失敗。頼みはスタンドアロンが印（MATERIAL_REQUEST）を名乗り、こちらも
/// 名乗っているときだけ来る。同じマテリアルの同じ頼みは 1 つにまとまる。
///
/// # Safety
/// `request` は 1 つ分の領域、`slot` は `slot_cap` バイトの書ける領域（または null）を指すこと。
#[no_mangle]
pub unsafe extern "C" fn ylb_next_request(
    handle: u64,
    request: *mut YlbRequest,
    slot: *mut u8,
    slot_cap: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let Some(s) = session(handle) else {
            return YLB_E_HANDLE;
        };
        if request.is_null() {
            return YLB_E_ARGUMENT;
        }
        let Some(r) = s.state().requests.pop_front() else {
            return 0;
        };
        put_text_written(&r.item.slot, slot, slot_cap);
        *request = YlbRequest {
            generation: r.generation,
            material: r.item.material,
            wants: r.item.wants as u32,
            slot_len: r.item.slot.len() as i32,
            have: r.item.have,
        };
        1
    })
}

/// まだ送り終えていない命令のバイトの合計（順番待ちに積んだものと、いま書いている途中のもの）。大きな絵を続けて送るとき、これが小さくなるまで
/// 次を積まないための目安。
#[no_mangle]
pub extern "C" fn ylb_pending_bytes(handle: u64) -> u64 {
    guard(0, || match session(handle) {
        Some(s) => s.pending_bytes(),
        None => 0,
    })
}

/// いま積める大きさの目安（バイト）。送りの列の上限（256 MiB）から、積んである量を引いたもの。何も積んでいなければ、1 つの命令が上限より大きくても
/// 入るので `u64::MAX`。数が合わないハンドルは 0。C# は、大きな絵を読む前にこれで足りるかを見て、足りなければ読まずに待つ（Unity の主のスレッドは
/// 待たない）。置き換えで空く分は数えないので、足りなくても `ylb_*_send` が積めることはある。
#[no_mangle]
pub extern "C" fn ylb_send_room(handle: u64) -> u64 {
    guard(0, || match session(handle) {
        Some(s) => s.send_room(),
        None => 0,
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

/// チャンネルの全部のタイルを汚れたことにする（Unity 側のテクスチャを作り直した・失ったとき、次の ylb_copy_dirty で全面を写し直す）。
#[no_mangle]
pub extern "C" fn ylb_channel_mark_all_dirty(handle: u64, set: u32, channel: i32) -> i32 {
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
        c.dirty.iter_mut().for_each(|d| *d = true);
        c.dirty_count = c.dirty.len() as u32;
        let dirty = c.dirty_count as i32;
        st.serial += 1;
        dirty
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
/// image は null でもよい（strip が要る）: 全体を GPU のテクスチャ（RenderTexture）に持つ側が、CPU に全体の写しを持たずに帯だけを受ける。
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
        if (image.is_null() || image_len == 0) && strip.is_null() {
            return YLB_E_ARGUMENT;
        }
        let image = (!image.is_null() && image_len > 0)
            .then(|| std::slice::from_raw_parts_mut(image, image_len as usize));
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

/// 自己診断のスタンドアロンが、つながりから読むのを止める（`paused` が 0 以外）か、再び読む（0）。止めている間、相手の書く枠は溜まり、ブリッジの
/// 送りの列は読まれない相手の様子になる（上限と「混んでいる」の確かめ用）。
#[no_mangle]
pub extern "C" fn ylb_test_server_pause_reading(server: u64, paused: i32) -> i32 {
    guard(YLB_E_PANIC, || {
        match SERVERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&server)
        {
            Some(s) => {
                s.pause_reading(paused != 0);
                0
            }
            None => YLB_E_HANDLE,
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

/// 自己診断のスタンドアロンの鍵のファイルを、別の鍵に差し替える（つなぎ直すブリッジは、鍵の合わない断りを受ける）。
#[no_mangle]
pub extern "C" fn ylb_test_server_replace_key(server: u64) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        match servers.get(&server) {
            Some(s) => s.replace_key(),
            None => YLB_E_HANDLE,
        }
    })
}

/// 自己診断のスタンドアロンの名乗りを決める（次につなぐブリッジから効く）。`app_version`・`min_peer` は `major << 32 | minor << 16 | patch`
/// （`u64::MAX` は版を名乗らない古いスタンドアロンの役）、`features` は出す機能の印。既定はこの DLL の版・要求なし・印なし。
#[no_mangle]
pub extern "C" fn ylb_test_server_configure(
    server: u64,
    app_version: u64,
    min_peer: u64,
    features: u64,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        match servers.get(&server) {
            Some(s) => {
                s.configure(
                    AppVersion::unpack(app_version),
                    AppVersion::unpack(min_peer).unwrap_or(AppVersion::ZERO),
                    features,
                );
                0
            }
            None => YLB_E_HANDLE,
        }
    })
}

/// 自己診断のスタンドアロンの読めるプロトコルの版の範囲を決める（次につなぐブリッジから効く。ブリッジの範囲と重ならなければ、版の範囲の断りを返す）。
#[no_mangle]
pub extern "C" fn ylb_test_server_set_protocol(server: u64, min: u32, max: u32) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        match servers.get(&server) {
            Some(s) if min <= max && max <= u16::MAX as u32 => {
                s.set_protocol_range(min as u16, max as u16);
                0
            }
            Some(_) => YLB_E_ARGUMENT,
            None => YLB_E_HANDLE,
        }
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

/// 自己診断のスタンドアロンが最後に受けた、マテリアル `material` の値のプロパティ `name` を `out`（4 つの f32。数は x、Int は x に
/// 数として）へ写す。返すのは型（0 Float・1 Int・2 Color・3 Vector）、無ければ YLB_E_ARGUMENT。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_value(
    server: u64,
    material: u32,
    name: *const u8,
    name_len: i32,
    out: *mut f32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = text(name, name_len) else {
            return YLB_E_ARGUMENT;
        };
        if out.is_null() {
            return YLB_E_ARGUMENT;
        }
        let (kind, v) = match s.value(material, name) {
            Some(PropertyValue::Float(x)) => (0, [x, 0.0, 0.0, 0.0]),
            Some(PropertyValue::Int(x)) => (1, [x as f32, 0.0, 0.0, 0.0]),
            Some(PropertyValue::Color(c)) => (2, c),
            Some(PropertyValue::Vector(c)) => (3, c),
            None => return YLB_E_ARGUMENT,
        };
        std::ptr::copy_nonoverlapping(v.as_ptr(), out, 4);
        kind
    })
}

/// 自己診断のスタンドアロンが最後に受けた、マテリアル `material` のスロット `name` の様子（0〜4。`ylb_values_slot` と同じ番号）。
/// キーワードを引くときは `keyword` を 0 でなくする（あれば 1、無ければ 0）。値が無ければ YLB_E_ARGUMENT。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_slot(
    server: u64,
    material: u32,
    name: *const u8,
    name_len: i32,
    keyword: i32,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        let Some(name) = text(name, name_len) else {
            return YLB_E_ARGUMENT;
        };
        if keyword != 0 {
            return s.has_keyword(material, name) as i32;
        }
        s.slot(material, name)
            .map_or(YLB_E_ARGUMENT, |st| st as i32)
    })
}

/// 自己診断のスタンドアロンが最後に受けた、マテリアル `material` のスロット `slot` の元の絵の様子。無ければ YLB_E_ARGUMENT。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_original(
    server: u64,
    material: u32,
    slot: *const u8,
    slot_len: i32,
    out: *mut YlbTestServerOriginal,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        let Some(slot) = text(slot, slot_len) else {
            return YLB_E_ARGUMENT;
        };
        if out.is_null() {
            return YLB_E_ARGUMENT;
        }
        match s.original(material, slot) {
            Some(t) => {
                *out = t;
                0
            }
            None => YLB_E_ARGUMENT,
        }
    })
}

/// 自己診断のスタンドアロンから、つながっているブリッジへ頼みを 1 つ送る（スタンドアロンが頼む役）。`generation` が 0 なら今のモデルの世代、
/// `wants` は bit0 値・bit1 元の絵、`slot` は元の絵のスロット（値だけなら空でよい）、`have` は手元の絵の印（0 は持たない）。
/// 相手に印（MATERIAL_REQUEST）が無い・つながっていなければ YLB_E_STATE。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_request(
    server: u64,
    generation: u32,
    material: u32,
    wants: i32,
    slot: *const u8,
    slot_len: i32,
    have: u64,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        let Some(slot) = (if slot_len == 0 {
            Some("")
        } else {
            text(slot, slot_len)
        }) else {
            return YLB_E_ARGUMENT;
        };
        if !(1..=255).contains(&wants) {
            return YLB_E_ARGUMENT;
        }
        s.request(generation, material, wants as u8, slot, have)
    })
}

/// 自己診断のスタンドアロンが最後に受けた、マテリアル `material` のスロット `slot` の絵の様子。無ければ YLB_E_ARGUMENT。
#[no_mangle]
pub unsafe extern "C" fn ylb_test_server_texture(
    server: u64,
    material: u32,
    slot: *const u8,
    slot_len: i32,
    out: *mut YlbTestServerTexture,
) -> i32 {
    guard(YLB_E_PANIC, || {
        let servers = SERVERS.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = servers.get(&server) else {
            return YLB_E_HANDLE;
        };
        let Some(slot) = text(slot, slot_len) else {
            return YLB_E_ARGUMENT;
        };
        if out.is_null() {
            return YLB_E_ARGUMENT;
        }
        match s.texture(material, slot) {
            Some(t) => {
                *out = t;
                0
            }
            None => YLB_E_ARGUMENT,
        }
    })
}
