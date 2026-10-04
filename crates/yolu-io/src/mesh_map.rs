//! .ylp 内の meshmap-*.bin。形式1〜3を読み、形式3で書く。
use crate::{check, check_budget, Error, Result};
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use std::io::{Read, Write};
use yolu_core::mesh_maps::{BakedMeshMap, MeshMapKind, MeshMapProvenance};

pub const FORMAT_VERSION: i32 = 3;
pub fn entry_name(kind: MeshMapKind) -> String {
    format!("meshmap-{}.bin", kind.name())
}
pub fn parse_entry_name(name: &str) -> Option<MeshMapKind> {
    MeshMapKind::ALL
        .into_iter()
        .find(|k| entry_name(*k) == name)
}
fn core_error(e: yolu_core::mesh_maps::MeshMapError) -> Error {
    Error::InvalidData(e.to_string())
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        check(n <= self.0.len(), "メッシュマップが途中で切れています")?;
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Ok(a)
    }
    fn int(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn double(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn string(&mut self) -> Result<String> {
        let n = self.int()?;
        check_budget(
            (0..=4096).contains(&n),
            "メッシュマップの文字列が長すぎます",
        )?;
        String::from_utf8(self.take(n as usize)?.to_vec())
            .map_err(|_| Error::InvalidData("メッシュマップの UTF-8 が不正です".into()))
    }
}
fn int(out: &mut Vec<u8>, n: i32) {
    out.extend(n.to_le_bytes());
}
fn string(out: &mut Vec<u8>, s: &str) {
    int(out, s.len() as i32);
    out.extend(s.as_bytes());
}
pub fn write(map: &BakedMeshMap) -> Result<Vec<u8>> {
    let p = map.provenance();
    p.validate().map_err(core_error)?;
    let mut out = b"YLPMMAP\0".to_vec();
    for n in [FORMAT_VERSION, p.kind as i32, p.engine_version] {
        int(&mut out, n);
    }
    string(&mut out, &p.mesh_hash);
    string(&mut out, &p.topology_hash);
    for n in [
        p.uv_channel,
        p.width,
        p.height,
        p.target_slot,
        p.padding,
        p.antialiasing,
        map.channels() as i32,
        p.target_slots.len() as i32,
    ] {
        int(&mut out, n);
    }
    for &n in &p.target_slots {
        int(&mut out, n);
    }
    for s in [&p.settings_key, &p.space, &p.pose, &p.source] {
        string(&mut out, s);
    }
    for n in p.bounds_min.into_iter().chain(p.bounds_max) {
        out.extend(n.to_le_bytes());
    }
    let texels = map.coverage().len();
    let mut raw = vec![0; texels * (1 + 2 * map.channels())];
    raw[..texels].copy_from_slice(map.coverage());
    for c in 0..map.channels() {
        let high = texels * (1 + 2 * c);
        let low = high + texels;
        for y in 0..map.height() {
            let mut previous = 0u16;
            for x in 0..map.width() {
                let i = y * map.width() + x;
                let value = map.data()[i * map.channels() + c];
                let delta = value.wrapping_sub(previous);
                previous = value;
                raw[high + i] = (delta >> 8) as u8;
                raw[low + i] = delta as u8;
            }
        }
    }
    // Unity 同梱 Mono は CompressionLevel.Fastest でも zlib の既定レベル6を使う。
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::new(6));
    encoder.write_all(&raw)?;
    let payload = encoder.finish()?;
    int(&mut out, payload.len() as i32);
    out.extend(payload);
    Ok(out)
}
pub fn read(bytes: &[u8]) -> Result<BakedMeshMap> {
    read_with_limit(bytes, usize::MAX)
}
/// 展開用の面と返却用の正本の合計を、割り当て前に予算と照合する。
pub fn read_with_limit(bytes: &[u8], max_bytes: usize) -> Result<BakedMeshMap> {
    let mut r = Reader(bytes);
    check(
        r.take(8)? == b"YLPMMAP\0",
        "メッシュマップの識別子が違います",
    )?;
    let version = r.int()?;
    check(
        (1..=FORMAT_VERSION).contains(&version),
        "未対応のメッシュマップ形式です",
    )?;
    let kind = MeshMapKind::try_from(r.int()?).map_err(core_error)?;
    let engine_version = r.int()?;
    let mesh_hash = r.string()?;
    let topology_hash = r.string()?;
    let uv_channel = r.int()?;
    let width = r.int()?;
    let height = r.int()?;
    let target_slot = r.int()?;
    let padding = r.int()?;
    let antialiasing = if version >= 2 { r.int()? } else { 1 };
    check(
        r.int()? == kind.channels() as i32,
        "メッシュマップのチャンネル数が種類と一致しません",
    )?;
    let target_slots = if version >= 3 {
        let n = r.int()?;
        check(
            (0..=65536).contains(&n) && n as usize <= r.0.len() / 4,
            "メッシュマップのスロット数が範囲外です",
        )?;
        (0..n).map(|_| r.int()).collect::<Result<Vec<_>>>()?
    } else if target_slot >= 0 {
        vec![target_slot]
    } else {
        vec![]
    };
    let settings_key = r.string()?;
    let space = r.string()?;
    let pose = r.string()?;
    let source = r.string()?;
    let mut bounds_min = [0.; 3];
    let mut bounds_max = [0.; 3];
    for n in bounds_min.iter_mut().chain(bounds_max.iter_mut()) {
        *n = r.double()?;
    }
    let p = MeshMapProvenance {
        kind,
        engine_version,
        mesh_hash,
        topology_hash,
        uv_channel,
        width,
        height,
        target_slot,
        target_slots,
        padding,
        antialiasing,
        settings_key,
        space,
        pose,
        source,
        bounds_min,
        bounds_max,
    };
    p.validate().map_err(core_error)?;
    let length = r.int()?;
    check(length >= 0, "圧縮データの長さが負です")?;
    let payload = r.take(length as usize)?;
    check(
        r.0.is_empty(),
        "メッシュマップの末尾に余分なデータがあります",
    )?;
    let texels = width as usize * height as usize;
    let expected = texels * (1 + 2 * kind.channels());
    check_budget(
        expected * 2 <= max_bytes,
        "メッシュマップの展開がメモリ予算を超えます",
    )?;
    let mut raw = vec![0; expected];
    let mut decoder = DeflateDecoder::new(payload);
    decoder.read_exact(&mut raw)?;
    check(
        decoder.read(&mut [0u8; 1])? == 0,
        "メッシュマップの展開サイズが宣言を超えます",
    )?;
    check(
        raw[..texels].iter().all(|b| *b <= 3),
        "メッシュマップに未知のテクセルの由来があります",
    )?;
    let mut data = vec![0u16; texels * kind.channels()];
    for c in 0..kind.channels() {
        let high = texels * (1 + 2 * c);
        let low = high + texels;
        for y in 0..height as usize {
            let mut previous = 0u16;
            for x in 0..width as usize {
                let i = y * width as usize + x;
                previous = previous.wrapping_add(u16::from_be_bytes([raw[high + i], raw[low + i]]));
                data[i * kind.channels() + c] = previous;
            }
        }
    }
    BakedMeshMap::new(p, data, raw[..texels].to_vec()).map_err(core_error)
}
