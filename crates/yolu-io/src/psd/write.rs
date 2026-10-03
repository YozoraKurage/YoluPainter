use super::{binary::Emit, *};
use crate::{check, Result};
use std::collections::HashSet;
const DIVIDER: &str = "</Layer group>";
pub fn write_edited(origin: &ReadResult, edited: &Document, limits: &Limits) -> Result<Vec<u8>> {
    check(
        origin.mode == CompatibilityMode::EditableRaster,
        "PreserveOnly・Rejected 原本への編集書き戻しは禁止です",
    )?;
    write(edited, limits)
}
#[derive(Clone, Copy)]
struct Record<'a> {
    layer: &'a Layer,
    divider: bool,
}
impl Record<'_> {
    fn name(&self) -> &str {
        if self.divider {
            DIVIDER
        } else {
            &self.layer.name
        }
    }
    fn id(&self) -> i32 {
        if self.divider {
            match self.layer.kind {
                LayerKind::Group { divider_id, .. } => divider_id,
                _ => 0,
            }
        } else {
            self.layer.id
        }
    }
    fn mask(&self) -> Option<&Mask> {
        if self.divider {
            None
        } else {
            self.layer.mask.as_ref()
        }
    }
    fn raster(&self) -> bool {
        !self.divider && self.layer.kind == LayerKind::Raster
    }
}
fn flatten<'a>(
    layers: &'a [Layer],
    depth: usize,
    out: &mut Vec<Record<'a>>,
    ids: &mut HashSet<i32>,
    limits: &Limits,
) -> Result<()> {
    for l in layers.iter().rev() {
        check(
            l.id > 0 && ids.insert(l.id),
            "PSD IDは正の一意な整数が必要です",
        )?;
        check(
            !l.name.contains('\0') && l.name.encode_utf16().count() <= limits.max_name_code_units,
            "PSD の名前が不正、または長すぎます",
        )?;
        check(l.locks & !0x80000007 == 0, "未対応のロックビット")?;
        if let Some(m) = &l.mask {
            rectangle(m.left, m.top, m.width, m.height, limits)?;
            check(
                matches!(m.default_color, 0 | 255)
                    && m.pixels.len() as u64 == u64::from(m.width) * u64::from(m.height),
                "マスク値・画素数が不正です",
            )?
        }
        match &l.kind {
            LayerKind::Group {
                children,
                divider_id,
            } => {
                check(depth < limits.max_group_depth, "グループ深さの予算超過")?;
                check(
                    *divider_id >= 0 && (*divider_id == 0 || ids.insert(*divider_id)),
                    "区切りIDが不正・重複しています",
                )?;
                out.push(Record {
                    layer: l,
                    divider: true,
                });
                check(out.len() <= limits.max_layers, "レイヤー記録数の予算超過")?;
                flatten(children, depth + 1, out, ids, limits)?
            }
            LayerKind::Raster => {
                check(
                    l.blend_mode != BlendMode::PassThrough,
                    "通過モードはグループ専用です",
                )?;
                rectangle(l.left, l.top, l.width, l.height, limits)?;
                check(
                    l.width > 0
                        && l.height > 0
                        && l.pixels_rgba.len() as u64
                            == u64::from(l.width) * u64::from(l.height) * 4,
                    "ラスター画素数が矩形と不一致です",
                )?
            }
            LayerKind::Adjustment(a) => {
                validate_adjustment(a)?;
                check(
                    l.blend_mode != BlendMode::PassThrough,
                    "調整に通過モードは使えません",
                )?
            }
            LayerKind::SolidColor(_) => check(
                l.blend_mode != BlendMode::PassThrough,
                "塗りつぶしに通過モードは使えません",
            )?,
        }
        out.push(Record {
            layer: l,
            divider: false,
        });
        check(out.len() <= limits.max_layers, "レイヤー記録数の予算超過")?;
    }
    Ok(())
}
fn rectangle(x: i32, y: i32, w: u32, h: u32, l: &Limits) -> Result<()> {
    check(
        w <= l.max_dimension
            && h <= l.max_dimension
            && u64::from(w) * u64::from(h) <= l.max_canvas_pixels
            && i64::from(x) + i64::from(w) <= i64::from(i32::MAX)
            && i64::from(y) + i64::from(h) <= i64::from(i32::MAX),
        "PSD の矩形が不正、または予算超過です",
    )
}
fn validate_adjustment(a: &Adjustment) -> Result<()> {
    check(
        match *a {
            Adjustment::Invert => true,
            Adjustment::Levels {
                input_black: ib,
                input_white: iw,
                output_black: ob,
                output_white: ow,
                gamma: g,
            } => {
                ib <= 253
                    && (2..=255).contains(&iw)
                    && iw > ib
                    && ob <= 255
                    && ow <= 255
                    && (10..=999).contains(&g)
            }
            Adjustment::HueSaturation {
                hue: h,
                saturation: s,
                lightness: l,
            } => {
                (-180..=180).contains(&h) && (-100..=100).contains(&s) && (-100..=100).contains(&l)
            }
        },
        "調整がPSDの刻み・範囲に収まりません",
    )
}
pub(super) fn validate(d: &Document, limits: &Limits) -> Result<()> {
    preflight(d, limits).map(|_| ())
}
fn preflight<'a>(d: &'a Document, limits: &Limits) -> Result<(Vec<Record<'a>>, usize)> {
    limits.validate()?;
    rectangle(0, 0, d.width, d.height, limits)?;
    check(
        d.width > 0 && d.height > 0 && !d.layers.is_empty(),
        "PSD は空のキャンバス・レイヤー一覧を持てません",
    )?;
    let canvas = u64::from(d.width) * u64::from(d.height) * 4;
    check(
        d.composite_rgba
            .as_ref()
            .is_none_or(|p| p.len() as u64 == canvas),
        "統合RGBAの大きさが不一致です",
    )?;
    let mut records = Vec::new();
    flatten(&d.layers, 0, &mut records, &mut HashSet::new(), limits)?;
    let mut pixels = canvas;
    let mut metadata = 0u64;
    let mut info = 2u64;
    for r in &records {
        let units = r.name().encode_utf16().count() as u64;
        let pascal = (units.min(255) + 1).div_ceil(4) * 4;
        let section = if r.divider {
            16
        } else {
            match &r.layer.kind {
                LayerKind::Raster => 0,
                LayerKind::Group { .. } => 24,
                LayerKind::Adjustment(a) => 12 + adjustment_block(a).1.len() as u64,
                LayerKind::SolidColor(c) => 12 + solid_block(*c).len() as u64,
            }
        };
        let extra = 8
            + pascal
            + 16
            + units * 2
            + if r.id() != 0 { 16 } else { 0 }
            + if !r.divider && r.layer.locks != 0 {
                16
            } else {
                0
            }
            + section
            + if r.mask().is_some() { 20 } else { 0 };
        let layer = if r.raster() {
            u64::from(r.layer.width) * u64::from(r.layer.height) * 4
        } else {
            0
        };
        let mask = r.mask().map_or(0, |m| m.pixels.len() as u64);
        pixels += layer + mask;
        metadata += extra;
        info += 58 + extra + 8 + layer + if r.mask().is_some() { 8 + mask } else { 0 };
    }
    check(
        pixels <= limits.max_decoded_bytes && metadata <= limits.max_metadata_bytes as u64,
        "PSD の画素・メタデータ予算超過",
    )?;
    info += info & 1;
    let total = 26 + 4 + 4 + 4 + 4 + info + 4 + 2 + canvas;
    check(
        total <= limits.max_output_bytes as u64 && total <= i32::MAX as u64,
        "PSD 出力予算超過",
    )?;
    Ok((records, total as usize))
}
pub fn write(d: &Document, limits: &Limits) -> Result<Vec<u8>> {
    let (records, total) = preflight(d, limits)?;
    let mut info = Vec::new();
    info.u16((-(records.len() as i16)) as u16);
    for r in &records {
        let l = r.layer;
        if r.raster() {
            bounds(&mut info, l.left, l.top, l.width, l.height)
        } else {
            info.extend([0; 16])
        }
        info.u16(if r.mask().is_some() { 5 } else { 4 });
        let len = if r.raster() {
            l.width * l.height + 2
        } else {
            2
        };
        for id in [0, 1, 2, -1i16] {
            info.u16(id as u16);
            info.u32(len)
        }
        if let Some(m) = r.mask() {
            info.u16((-2i16) as u16);
            info.u32(m.width * m.height + 2)
        }
        info.extend(b"8BIM");
        if r.divider {
            info.extend(b"norm");
            info.extend([255, 0, 0, 0])
        } else {
            info.extend(if l.blend_mode == BlendMode::PassThrough {
                *b"norm"
            } else {
                l.blend_mode.key()
            });
            let flag = u8::from(!l.visible) * 2
                + u8::from(l.locks & 1 != 0 && l.locks & 0x80000000 == 0)
                + if matches!(l.kind, LayerKind::Adjustment(_) | LayerKind::SolidColor(_)) {
                    24
                } else {
                    0
                };
            info.extend([l.opacity, u8::from(l.clipping), flag, 0])
        }
        let mut e = Vec::new();
        if let Some(m) = r.mask() {
            e.u32(20);
            bounds(&mut e, m.left, m.top, m.width, m.height);
            e.push(m.default_color);
            e.push(u8::from(!m.enabled) * 2 + if m.density != 255 { 16 } else { 0 });
            if m.density != 255 {
                e.extend([1, m.density])
            } else {
                e.extend([0; 2])
            }
        } else {
            e.u32(0)
        }
        e.u32(0);
        let units: Vec<_> = r.name().encode_utf16().collect();
        let n = units.len().min(255);
        e.push(n as u8);
        e.extend(
            units[..n]
                .iter()
                .map(|v| if *v <= 127 { *v as u8 } else { b'?' }),
        );
        e.extend(vec![0; (4 - (n + 1) % 4) % 4]);
        let mut name = Vec::new();
        name.u32(units.len() as u32);
        for v in units {
            name.u16(v)
        }
        e.tag(b"luni", &name);
        if r.id() != 0 {
            e.tag(b"lyid", &r.id().to_be_bytes())
        }
        if !r.divider && l.locks != 0 {
            let bits = if l.locks & 0x80000000 != 0 {
                0x80000000u32
            } else {
                l.locks
            };
            e.tag(b"lspf", &bits.to_be_bytes())
        }
        if r.divider {
            e.tag(b"lsct", &3u32.to_be_bytes())
        } else {
            match &l.kind {
                LayerKind::Raster => {}
                LayerKind::Group { .. } => {
                    let mut body = Vec::new();
                    body.u32(1);
                    body.extend(b"8BIM");
                    body.extend(l.blend_mode.key());
                    e.tag(b"lsct", &body)
                }
                LayerKind::Adjustment(a) => {
                    let (key, body) = adjustment_block(a);
                    e.tag(&key, &body)
                }
                LayerKind::SolidColor(c) => e.tag(b"SoCo", &solid_block(*c)),
            }
        }
        info.section(&e);
    }
    for r in &records {
        for c in 0..4 {
            info.u16(0);
            if r.raster() {
                info.extend(r.layer.pixels_rgba.iter().skip(c).step_by(4))
            }
        }
        if let Some(m) = r.mask() {
            info.u16(0);
            info.extend(&m.pixels)
        }
    }
    if info.len() % 2 != 0 {
        info.push(0)
    }
    let mut lm = Vec::new();
    lm.section(&info);
    lm.u32(0);
    let mut out = Vec::with_capacity(total);
    out.extend(b"8BPS");
    out.u16(1);
    out.extend([0; 6]);
    out.u16(4);
    out.u32(d.height);
    out.u32(d.width);
    out.u16(8);
    out.u16(3);
    out.u32(0);
    out.u32(0);
    out.section(&lm);
    out.u16(0);
    let mut merged = d
        .composite_rgba
        .clone()
        .unwrap_or_else(|| super::composite::composite(d));
    super::composite::matte(&mut merged);
    for c in 0..4 {
        out.extend(merged.iter().skip(c).step_by(4))
    }
    check(out.len() == total, "PSD 書き出し長と事前検証が不一致です")?;
    Ok(out)
}
fn bounds(w: &mut Vec<u8>, x: i32, y: i32, width: u32, height: u32) {
    w.u32(y as u32);
    w.u32(x as u32);
    w.u32((i64::from(y) + i64::from(height)) as u32);
    w.u32((i64::from(x) + i64::from(width)) as u32)
}
fn adjustment_block(a: &Adjustment) -> ([u8; 4], Vec<u8>) {
    let mut b = Vec::new();
    match *a {
        Adjustment::Invert => (*b"nvrt", b),
        Adjustment::Levels {
            input_black: ib,
            input_white: iw,
            output_black: ob,
            output_white: ow,
            gamma: g,
        } => {
            b.u16(2);
            for v in [ib, iw, ob, ow, g] {
                b.u16(v)
            }
            for _ in 1..29 {
                for v in [0, 255, 0, 255, 100] {
                    b.u16(v)
                }
            }
            (*b"levl", b)
        }
        Adjustment::HueSaturation {
            hue,
            saturation,
            lightness,
        } => {
            b.u16(2);
            b.extend([0; 2]);
            for v in [0, 25, 0, hue, saturation, lightness] {
                b.u16(v as u16)
            }
            for range in super::read::HUE_RANGES {
                for v in range {
                    b.u16(v as u16)
                }
                b.extend([0; 6])
            }
            for i in 0..6 {
                for v in [i * 60, 100, 50] {
                    b.u16(v)
                }
            }
            (*b"hue2", b)
        }
    }
}
fn solid_block(c: [u8; 3]) -> Vec<u8> {
    fn name(w: &mut Vec<u8>) {
        w.u32(1);
        w.u16(0)
    }
    fn key(w: &mut Vec<u8>, k: &[u8; 4]) {
        w.u32(0);
        w.extend(k)
    }
    let mut w = Vec::new();
    w.u32(16);
    name(&mut w);
    key(&mut w, b"null");
    w.u32(1);
    key(&mut w, b"Clr ");
    w.extend(b"Objc");
    name(&mut w);
    key(&mut w, b"RGBC");
    w.u32(3);
    for (k, v) in [b"Rd  ", b"Grn ", b"Bl  "].iter().zip(c) {
        key(&mut w, k);
        w.extend(b"doub");
        w.extend(f64::from(v).to_be_bytes())
    }
    w
}
