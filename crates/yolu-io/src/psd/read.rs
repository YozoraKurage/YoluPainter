use super::{binary::Reader, *};
use crate::{check, check_budget, Error, Result};
use std::collections::HashSet;
use std::io::Read;

struct State<'a> {
    limits: &'a Limits,
    notes: Vec<Diagnostic>,
    unsupported: bool,
    metadata: usize,
    pixels: u64,
    omitted: Vec<(String, usize, usize, usize)>,
}
impl State<'_> {
    fn note(&mut self, code: &str, message: impl Into<String>, offset: usize, length: usize) {
        if self.notes.len() < self.limits.max_diagnostics {
            self.notes.push(Diagnostic {
                code: code.into(),
                message: message.into(),
                offset,
                length,
            })
        }
    }
    fn preserve(&mut self, code: &str, message: impl Into<String>, offset: usize, length: usize) {
        self.unsupported = true;
        self.note(code, message, offset, length)
    }
    fn omitted(&mut self, what: impl Into<String>, offset: usize, length: usize) {
        let what = what.into();
        if let Some(x) = self.omitted.iter_mut().find(|x| x.0 == what) {
            x.3 += 1
        } else {
            self.omitted.push((what, offset, length, 1))
        }
    }
    fn metadata(&mut self, n: usize) -> Result<()> {
        self.metadata = self
            .metadata
            .checked_add(n)
            .ok_or_else(|| Error::InvalidData("メタデータ長のオーバーフロー".into()))?;
        check_budget(
            self.metadata <= self.limits.max_metadata_bytes,
            "PSD のメタデータ予算超過",
        )
    }
    fn pixels(&mut self, n: u64) -> Result<()> {
        self.pixels = self
            .pixels
            .checked_add(n)
            .ok_or_else(|| Error::InvalidData("画素長のオーバーフロー".into()))?;
        check_budget(
            self.pixels <= self.limits.max_decoded_bytes,
            "PSD の復号予算超過",
        )
    }
}
fn rejected(original: Option<Vec<u8>>, code: &str, message: String) -> ReadResult {
    ReadResult {
        mode: CompatibilityMode::Rejected,
        document: None,
        original,
        diagnostics: vec![Diagnostic {
            code: code.into(),
            message: message.clone(),
            offset: message
                .split_once(": ")
                .and_then(|(p, _)| p.parse().ok())
                .unwrap_or(0),
            length: 0,
        }],
    }
}
pub fn read(bytes: &[u8], limits: &Limits) -> Result<ReadResult> {
    limits.validate()?;
    if bytes.len() > limits.max_source_bytes {
        return Ok(rejected(
            None,
            "SourceLimit",
            "PSD 原本の保持予算超過".into(),
        ));
    }
    Ok(read_owned(bytes.to_vec(), limits))
}
/// 読み手を閉じず、原本の上限+1バイトまでで止める。
pub fn read_stream(reader: &mut impl Read, limits: &Limits) -> Result<ReadResult> {
    limits.validate()?;
    let mut b = Vec::new();
    reader
        .take(limits.max_source_bytes as u64 + 1)
        .read_to_end(&mut b)?;
    if b.len() > limits.max_source_bytes {
        Ok(rejected(
            None,
            "SourceLimit",
            "PSD 原本の保持予算超過".into(),
        ))
    } else {
        Ok(read_owned(b, limits))
    }
}
fn read_owned(bytes: Vec<u8>, limits: &Limits) -> ReadResult {
    let mut s = State {
        limits,
        notes: Vec::new(),
        unsupported: false,
        metadata: 0,
        pixels: 0,
        omitted: Vec::new(),
    };
    match parse(&bytes, &mut s) {
        Err(e) => rejected(Some(bytes), "MalformedOrLimit", e.to_string()),
        Ok(doc) => {
            for (what, offset, length, count) in std::mem::take(&mut s.omitted) {
                s.note(
                    "NotCarriedIntoExport",
                    format!(
                        "{what}（{count}件）は原本に保持しますが、編集後の書き出しには含まれません"
                    ),
                    offset,
                    length,
                )
            }
            ReadResult {
                mode: if s.unsupported {
                    CompatibilityMode::PreserveOnly
                } else {
                    CompatibilityMode::EditableRaster
                },
                document: if s.unsupported { None } else { doc },
                original: Some(bytes),
                diagnostics: s.notes,
            }
        }
    }
}
fn parse(bytes: &[u8], s: &mut State) -> Result<Option<Document>> {
    let mut r = Reader::new(bytes);
    check(r.key()? == *b"8BPS", "PSD シグネチャが不正です")?;
    let version = r.u16()?;
    check(version == 1 || version == 2, "PSD 版が不正です")?;
    r.zeros(6)?;
    let channels = r.u16()?;
    let height = r.u32()?;
    let width = r.u32()?;
    let depth = r.u16()?;
    let color = r.u16()?;
    let max = if version == 2 { 300000 } else { 30000 };
    check(
        (1..=56).contains(&channels) && width > 0 && height > 0 && width <= max && height <= max,
        "PSD 寸法・チャンネル数が不正です",
    )?;
    if version == 2 {
        s.preserve("PSB", "PSB は復号・編集せず原本全体を保持します", 4, 0);
        return Ok(None);
    }
    rect(width.into(), height.into(), s)?;
    if depth != 8 || color != 3 || !matches!(channels, 3 | 4) {
        s.preserve(
            "ColorFormat",
            "RGB8 の3・4チャンネル以外は原本保持のみです",
            12,
            0,
        );
        return Ok(None);
    }
    let colors = r.section()?;
    s.metadata(colors.remaining())?;
    if colors.remaining() != 0 {
        s.preserve(
            "ColorData",
            "RGB 色モードデータを解釈できません",
            colors.pos,
            colors.remaining(),
        )
    }
    resources(r.section()?, s)?;
    let mut lm = r.section()?;
    let mut records = Vec::new();
    let mut merged_alpha = false;
    if lm.remaining() == 0 {
        s.preserve("NoLayers", "レイヤーIDのない統合画像です", lm.pos, 0)
    } else {
        let mut info = lm.section()?;
        if info.remaining() == 0 {
            s.preserve("NoLayers", "レイヤー記録がありません", info.pos, 0)
        } else {
            let signed = info.i16()?;
            merged_alpha = signed < 0;
            let count = i32::from(signed).unsigned_abs() as usize;
            check_budget(count <= s.limits.max_layers, "PSD レイヤー数の予算超過")?;
            if count == 0 {
                s.preserve("NoLayers", "レイヤー記録がありません", info.pos - 2, 0)
            }
            let mut ids = HashSet::new();
            for _ in 0..count {
                let rec = record(&mut info, s)?;
                if !(rec.section == 3 && rec.layer.id == 0)
                    && (rec.layer.id <= 0 || !ids.insert(rec.layer.id))
                {
                    s.preserve(
                        "LayerIdentity",
                        "レイヤーIDが欠落・重複・不正です。名前で補修しません",
                        info.pos,
                        0,
                    )
                }
                records.push(rec)
            }
            for rec in &mut records {
                let l = &mut rec.layer;
                if !s.unsupported {
                    let n = u64::from(l.width) * u64::from(l.height) * 4;
                    s.pixels(n)?;
                    l.pixels_rgba = vec![0; n as usize];
                    for p in l.pixels_rgba.as_chunks_mut::<4>().0 {
                        p[3] = 255
                    }
                    if let Some(m) = &mut l.mask {
                        let n = u64::from(m.width) * u64::from(m.height);
                        s.pixels(n)?;
                        m.pixels = vec![0; n as usize]
                    }
                }
                for &(id, len) in &rec.channels {
                    let channel = info.slice(len)?;
                    if s.unsupported {
                        continue;
                    }
                    if id == -2 {
                        let m = l.mask.as_mut().unwrap();
                        if channel.remaining() == 0 && m.pixels.is_empty() {
                            continue;
                        }
                        decode(channel, m.width, m.height, &mut m.pixels, 0, 1, s)?
                    } else {
                        decode(
                            channel,
                            l.width,
                            l.height,
                            &mut l.pixels_rgba,
                            if id == -1 { 3 } else { id as usize },
                            4,
                            s,
                        )?
                    }
                }
            }
            if info.remaining() > 1 {
                s.preserve(
                    "LayerInfoTail",
                    "未知のレイヤー情報末尾",
                    info.pos,
                    info.remaining(),
                )
            } else {
                info.zeros(info.remaining())?
            }
        }
        let mask = lm.section()?;
        s.metadata(mask.remaining())?;
        if mask.remaining() != 0 {
            s.preserve(
                "GlobalMask",
                "全体マスクは未対応です",
                mask.pos,
                mask.remaining(),
            )
        }
        tags(lm, None, s)?;
    }
    let layers = tree(records, s)?;
    let doc = Document {
        width,
        height,
        layers,
        composite_rgba: None,
    };
    if channels == 4 && !merged_alpha {
        s.preserve(
            "ExtraAlpha",
            "4番目のチャンネルが統合透明度と宣言されていません",
            12,
            0,
        )
    }
    check(
        channels != 3 || !merged_alpha,
        "統合透明度の宣言に4番目のチャンネルがありません",
    )?;
    if r.remaining() < 2 {
        s.preserve("NoComposite", "統合画像がありません", r.pos, 0)
    }
    if !s.unsupported {
        let n = u64::from(width) * u64::from(height) * 4;
        s.pixels(n * 2)?;
        let mut merged = vec![0; n as usize];
        for p in merged.as_chunks_mut::<4>().0 {
            p[3] = 255
        }
        let offset = r.pos;
        decode_composite(r, width, height, channels as usize, &mut merged, s)?;
        if !s.unsupported {
            let mut expected = super::composite::composite(&doc);
            super::composite::matte(&mut expected);
            let worst = merged
                .iter()
                .zip(expected)
                .enumerate()
                .filter(|(i, _)| channels == 4 || i % 4 != 3)
                .map(|(_, (&a, b))| a.abs_diff(b))
                .max()
                .unwrap_or(0);
            if worst > 1 {
                if doc.layers.iter().any(|l| {
                    l.blend_mode != BlendMode::Normal
                        || l.clipping
                        || l.mask.is_some()
                        || l.kind != LayerKind::Raster
                }) {
                    s.note(
                        "CompositeDiffers",
                        format!("保存された統合画像と参照合成の最大差: {worst}/255"),
                        offset,
                        0,
                    )
                } else {
                    s.preserve(
                        "CompositeMismatch",
                        "通常合成と保存された統合画像が一致しません",
                        offset,
                        0,
                    )
                }
            }
        }
    }
    Ok(Some(doc))
}
fn rect(w: i64, h: i64, s: &State) -> Result<()> {
    check(
        w >= 0
            && h >= 0
            && w <= i64::from(s.limits.max_dimension)
            && h <= i64::from(s.limits.max_dimension)
            && (w as u64) * (h as u64) <= s.limits.max_canvas_pixels,
        "PSD の矩形が不正、または予算超過です",
    )
}
struct Record {
    layer: Layer,
    channels: Vec<(i16, usize)>,
    section: i32,
    section_key: Option<[u8; 4]>,
    subtype: i32,
    unknown_section: bool,
    adjustment_seen: bool,
    fill_seen: bool,
    protection: u32,
}
fn record(r: &mut Reader, s: &mut State) -> Result<Record> {
    let offset = r.pos;
    let top = r.i32()?;
    let left = r.i32()?;
    let h = i64::from(r.i32()?) - i64::from(top);
    let w = i64::from(r.i32()?) - i64::from(left);
    rect(w, h, s)?;
    let mut rec = Record {
        layer: Layer {
            top,
            left,
            width: w as u32,
            height: h as u32,
            ..Layer::default()
        },
        channels: Vec::new(),
        section: -1,
        section_key: None,
        subtype: 0,
        unknown_section: false,
        adjustment_seen: false,
        fill_seen: false,
        protection: 0,
    };
    let n = r.u16()?;
    check((1..=56).contains(&n), "レイヤーチャンネル数が不正です")?;
    let mut ids = HashSet::new();
    for _ in 0..n {
        let id = r.i16()?;
        let len = r.u32()? as usize;
        check(
            len <= i32::MAX as usize && ids.insert(id),
            "チャンネルの長さが不正、またはIDが重複しています",
        )?;
        if !(-2..=2).contains(&id) {
            s.preserve(
                if id == -3 {
                    "RealUserMask"
                } else {
                    "LayerChannel"
                },
                format!("未対応チャンネル {id}"),
                r.pos - 6,
                6,
            )
        }
        rec.channels.push((id, len))
    }
    check(r.key()? == *b"8BIM", "レイヤー合成のシグネチャが不正です")?;
    let key = r.key()?;
    rec.layer.opacity = r.u8()?;
    let clipping = r.u8()?;
    let flags = r.u8()?;
    rec.layer.visible = flags & 2 == 0;
    r.zeros(1)?;
    let mut extra = r.section()?;
    s.metadata(extra.remaining())?;
    rec.layer.mask = mask(extra.section()?, s)?;
    check(
        !ids.contains(&-2) || rec.layer.mask.is_some(),
        "マスクチャンネルにマスク定義がありません",
    )?;
    if let Some(m) = &rec.layer.mask {
        check(
            ids.contains(&-2) || m.width == 0 || m.height == 0,
            "マスク定義にチャンネルがありません",
        )?
    }
    let ranges = extra.section()?;
    let neutral = ranges.remaining() % 8 == 0
        && ranges.data[ranges.pos..ranges.end]
            .as_chunks::<4>()
            .0
            .iter()
            .all(|b| *b == [0, 0, 255, 255]);
    let n = extra.u8()? as usize;
    let name = extra.take(n)?;
    let ascii = name.is_ascii();
    rec.layer.name = name.iter().map(|b| char::from(*b)).collect();
    extra.zeros((4 - (n + 1) % 4) % 4)?;
    let unicode = tags(extra, Some(&mut rec), s)?;
    check_budget(
        rec.layer.name.encode_utf16().count() <= s.limits.max_name_code_units,
        "名前長の予算超過",
    )?;
    let divider = rec.section == 3;
    let folder = matches!(rec.section, 1 | 2);
    if rec.adjustment_seen && (divider || folder) {
        s.preserve(
            "Adjustment",
            "グループ・区切りに調整が付いています",
            offset,
            0,
        )
    }
    if rec.fill_seen && (divider || folder || rec.adjustment_seen) {
        s.preserve(
            "FillLayer",
            "グループ・区切り・調整に塗りつぶしが付いています",
            offset,
            0,
        )
    }
    if divider {
        if flags & 1 != 0 || rec.protection != 0 {
            s.omitted("区切りのロック", offset, 0)
        }
    } else {
        rec.layer.locks = (rec.protection & 0x80000007) | u32::from(flags & 1);
        if rec.protection & !0x80000007 != 0 {
            s.omitted("lspf の未対応ロックビット", offset, 0)
        }
    }
    if !neutral {
        s.preserve("BlendIf", "既定値以外の Blend-If", offset, 0)
    }
    if divider {
        if w != 0 || h != 0 {
            s.preserve("DividerPixels", "区切りの画素は未対応です", offset, 16)
        }
        if rec.layer.mask.is_some() || ids.contains(&-2) || ids.contains(&-3) {
            s.preserve("DividerMask", "区切りのマスクは未対応です", offset, 0)
        }
    } else {
        rec.layer.clipping = clipping == 1;
        if clipping > 1 {
            s.preserve("Clipping", "未知のクリッピング値", offset, 0)
        }
        if rec.layer.name.contains('\0') {
            s.preserve("NameNull", "名前中のNUL", offset, 0)
        }
        if !unicode && !ascii {
            s.preserve("LegacyNameEncoding", "Unicode名のない非ASCII名", offset, 0)
        }
        let mode = if folder {
            rec.section_key.unwrap_or(key)
        } else {
            key
        };
        match BlendMode::from_key(mode) {
            Some(m) if folder || m != BlendMode::PassThrough => rec.layer.blend_mode = m,
            _ => s.preserve(
                "BlendMode",
                format!("未対応の合成キー {}", String::from_utf8_lossy(&mode)),
                offset,
                4,
            ),
        }
        if folder {
            if rec.section == 2 {
                s.omitted("閉じたグループ", offset, 0)
            }
            if rec.subtype != 0 {
                s.omitted("シーングループ", offset, 0)
            }
            if w != 0 || h != 0 {
                s.preserve("GroupPixels", "グループ自身の画素", offset, 16)
            }
            if rec
                .section_key
                .is_some_and(|k| k != key && !(k == *b"pass" && key == *b"norm"))
            {
                s.preserve(
                    "GroupBlend",
                    "グループの合成キーが矛盾しています",
                    offset,
                    0,
                )
            }
            rec.layer.kind = LayerKind::Group {
                children: Vec::new(),
                divider_id: 0,
            };
        } else if rec.fill_seen {
            if w != 0 || h != 0 {
                s.omitted("SoCo の画素キャッシュ", offset, 16)
            }
        } else if rec.adjustment_seen {
            if w != 0 || h != 0 {
                s.preserve("AdjustmentPixels", "調整自身の画素", offset, 16)
            }
        } else {
            if w == 0 || h == 0 {
                s.preserve("EmptyLayer", "空のレイヤー", offset, 16)
            }
            if ![0, 1, 2].iter().all(|id| ids.contains(id)) {
                s.preserve("LayerChannels", "RGBチャンネルが不足しています", offset, 0)
            }
        }
    }
    let allowed = if divider || folder || rec.fill_seen || rec.adjustment_seen {
        1 | 2 | 8 | 16
    } else {
        1 | 2 | 8
    };
    if flags & !allowed != 0 {
        s.preserve("LayerFlags", "未知または画素無関係のフラグ", offset, 0)
    }
    Ok(rec)
}
fn tree(records: Vec<Record>, s: &State) -> Result<Vec<Layer>> {
    if records.iter().any(|r| r.unknown_section) {
        return Ok(records.into_iter().rev().map(|r| r.layer).collect());
    }
    let mut stack = Vec::new();
    let mut current = Vec::new();
    for r in records {
        match r.section {
            3 => {
                check_budget(
                    stack.len() < s.limits.max_group_depth,
                    "グループ深さの予算超過",
                )?;
                stack.push((std::mem::take(&mut current), r.layer.id))
            }
            1 | 2 => {
                let (outer, id) = stack
                    .pop()
                    .ok_or_else(|| Error::InvalidData("区切りのないグループ".into()))?;
                current.reverse();
                let mut l = r.layer;
                l.kind = LayerKind::Group {
                    children: current,
                    divider_id: id,
                };
                current = outer;
                current.push(l)
            }
            _ => current.push(r.layer),
        }
    }
    check(stack.is_empty(), "閉じていないグループ区切り")?;
    current.reverse();
    Ok(current)
}
fn mask(mut r: Reader, s: &mut State) -> Result<Option<Mask>> {
    if r.remaining() == 0 {
        return Ok(None);
    }
    let start = r.pos;
    let top = r.i32()?;
    let left = r.i32()?;
    let h = i64::from(r.i32()?) - i64::from(top);
    let w = i64::from(r.i32()?) - i64::from(left);
    rect(w, h, s)?;
    let color = r.u8()?;
    check(matches!(color, 0 | 255), "マスク既定値は0または255です")?;
    let flags = r.u8()?;
    let mut density = 255;
    for (bit, code) in [
        (1, "MaskPosition"),
        (4, "MaskInvert"),
        (8, "MaskFromRender"),
    ] {
        if flags & bit != 0 {
            s.preserve(code, "未対応のマスクフラグ", start, 0)
        }
    }
    if flags & !31 != 0 {
        s.preserve("MaskFlags", "未知のマスクフラグ", start, 0)
    }
    if flags & 16 != 0 {
        let p = r.u8()?;
        if p & 1 != 0 {
            density = r.u8()?
        }
        for (bit, n, code) in [
            (2, 8, "MaskFeather"),
            (4, 1, "VectorMaskDensity"),
            (8, 8, "VectorMaskFeather"),
        ] {
            if p & bit != 0 {
                r.take(n)?;
                s.preserve(code, "未対応のマスクパラメータ", start, 0)
            }
        }
        if p & !15 != 0 {
            s.preserve("MaskParameters", "未知のマスクパラメータ", start, 0);
            r.take(r.remaining())?;
        }
    }
    if r.remaining() >= 18 {
        s.preserve(
            "UserAndVectorMask",
            "ユーザーとベクターの複合マスク",
            r.pos,
            18,
        );
        r.take(18)?;
    }
    if r.take(r.remaining())?.iter().any(|b| *b != 0) {
        s.preserve("MaskTail", "未知のマスク末尾", start, 0)
    }
    Ok(Some(Mask {
        top,
        left,
        width: w as u32,
        height: h as u32,
        default_color: color,
        enabled: flags & 2 == 0,
        density,
        pixels: Vec::new(),
    }))
}
fn decode(
    mut r: Reader,
    w: u32,
    h: u32,
    out: &mut [u8],
    component: usize,
    stride: usize,
    s: &mut State,
) -> Result<()> {
    let compression = r.u16()?;
    match compression {
        0 => {
            check(
                r.remaining() == w as usize * h as usize,
                "raw チャンネル長が矩形と不一致です",
            )?;
            for i in 0..w as usize * h as usize {
                out[i * stride + component] = r.u8()?
            }
        }
        1 => {
            let mut table = r.slice(h as usize * 2)?;
            for y in 0..h as usize {
                let n = table.u16()? as usize;
                packbits(
                    r.slice(n)?,
                    w as usize,
                    out,
                    y * w as usize * stride + component,
                    stride,
                )?
            }
            check(r.remaining() == 0, "RLE チャンネルに余分なデータ")?
        }
        _ => s.preserve(
            "Compression",
            format!("未対応の圧縮 {compression}"),
            r.pos - 2,
            0,
        ),
    }
    Ok(())
}
fn decode_composite(
    mut r: Reader,
    w: u32,
    h: u32,
    channels: usize,
    out: &mut [u8],
    s: &mut State,
) -> Result<()> {
    let compression = r.u16()?;
    let n = w as usize * h as usize;
    match compression {
        0 => {
            check(
                r.remaining() == n * channels,
                "統合rawチャンネル長が不一致です",
            )?;
            for c in 0..channels {
                for i in 0..n {
                    out[i * 4 + c] = r.u8()?
                }
            }
        }
        1 => {
            let mut table = r.slice(channels * h as usize * 2)?;
            for c in 0..channels {
                for y in 0..h as usize {
                    let n = table.u16()? as usize;
                    packbits(r.slice(n)?, w as usize, out, y * w as usize * 4 + c, 4)?
                }
            }
            check(r.remaining() == 0, "統合RLEに余分なデータ")?
        }
        _ => s.preserve("CompositeCompression", "未対応の統合圧縮", r.pos - 2, 0),
    }
    Ok(())
}
fn packbits(mut r: Reader, w: usize, out: &mut [u8], offset: usize, stride: usize) -> Result<()> {
    let mut x = 0;
    while r.remaining() > 0 {
        let control = r.u8()? as i8;
        if control == -128 {
            continue;
        }
        let n = if control >= 0 {
            control as usize + 1
        } else {
            (1 - i16::from(control)) as usize
        };
        check(n <= w - x, "RLE 行が幅を超えています")?;
        if control >= 0 {
            for b in r.take(n)? {
                out[offset + x * stride] = *b;
                x += 1
            }
        } else {
            let b = r.u8()?;
            for _ in 0..n {
                out[offset + x * stride] = b;
                x += 1
            }
        }
    }
    check(x == w, "RLE 行が幅を満たしていません")
}
fn resources(mut r: Reader, s: &mut State) -> Result<()> {
    s.metadata(r.remaining())?;
    while r.remaining() > 0 {
        let start = r.pos;
        check(r.key()? == *b"8BIM", "画像リソースのシグネチャが不正です")?;
        let id = r.u16()?;
        let n = r.u8()? as usize;
        r.take(n)?;
        if !(n + 1).is_multiple_of(2) {
            r.zeros(1)?
        }
        let mut body = r.section()?;
        if body.remaining() % 2 != 0 {
            r.zeros(1)?
        }
        let accepted = match id {
            1039 => icc(&body).as_deref() == Some("sRGB IEC61966-2.1"),
            1064 => {
                if body.remaining() != 12 {
                    false
                } else {
                    body.u32()?;
                    f64::from_be_bytes(body.take(8)?.try_into().unwrap()) == 1.0
                }
            }
            1005
            | 1010
            | 1011
            | 1024
            | 1025
            | 1026
            | 1028
            | 1032..=1037
            | 1044
            | 1049
            | 1050
            | 1054
            | 1057..=1062
            | 1065
            | 1069
            | 1072
            | 1082
            | 1083
            | 1088
            | 2000..=2997
            | 7000
            | 7001
            | 8000
            | 10000 => true,
            _ => false,
        };
        if accepted {
            s.omitted(format!("画像リソース {id}"), start, r.pos - start)
        } else {
            s.preserve(
                "ImageResource",
                format!("色解釈または未知の画像リソース {id}"),
                start,
                r.pos - start,
            )
        }
    }
    Ok(())
}
fn icc(r: &Reader) -> Option<String> {
    fn parse(b: &[u8]) -> Option<String> {
        let u = |p: usize| {
            b.get(p..p + 4)
                .map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
        };
        if b.len() < 132
            || u(0)? > b.len()
            || b.get(36..40)? != b"acsp"
            || b.get(16..20)? != b"RGB "
        {
            return None;
        }
        let n = u(128)?;
        if n > 1000 || 132 + n * 12 > b.len() {
            return None;
        }
        for i in 0..n {
            let e = 132 + i * 12;
            if &b[e..e + 4] != b"desc" {
                continue;
            }
            let p = u(e + 4)?;
            let size = u(e + 8)?;
            let d = b.get(p..p.checked_add(size)?)?;
            if size < 12 {
                return None;
            }
            let du = |p: usize| {
                d.get(p..p + 4)
                    .map(|b| u32::from_be_bytes(b.try_into().unwrap()) as usize)
            };
            let text = match d.get(..4)? {
                b"desc" => {
                    let n = du(8)?;
                    if n < 1 {
                        return None;
                    }
                    d.get(12..12 + n)?
                        .iter()
                        .map(|b| if *b < 128 { char::from(*b) } else { '?' })
                        .collect::<String>()
                }
                b"mluc" => {
                    if du(8)? < 1 || du(12)? < 12 || 16 + du(12)? > size {
                        return None;
                    }
                    let len = du(20)?;
                    let off = du(24)?;
                    if len % 2 != 0 {
                        return None;
                    }
                    String::from_utf16_lossy(
                        &d.get(off..off.checked_add(len)?)?
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|b| u16::from_be_bytes([b[0], b[1]]))
                            .collect::<Vec<_>>(),
                    )
                }
                _ => return None,
            };
            return Some(text.trim_end_matches('\0').into());
        }
        None
    }
    parse(&r.data[r.pos..r.end])
}
fn tags(mut r: Reader, mut record: Option<&mut Record>, s: &mut State) -> Result<bool> {
    let mut unicode = false;
    let mut seen = HashSet::new();
    while r.remaining() > 0 {
        let start = r.pos;
        if r.remaining() < 12 {
            s.preserve("UnknownTail", "未知の追加情報末尾", start, r.remaining());
            break;
        }
        let sig = r.key()?;
        check(
            sig == *b"8BIM" || sig == *b"8B64",
            "タグのシグネチャが不正です",
        )?;
        let key = r.key()?;
        let mut b = r.section()?;
        let size = b.remaining();
        if record.is_none() {
            s.metadata(size + 12)?
        }
        if size % 2 != 0 {
            r.zeros(1)?
        }
        let length = r.pos - start;
        if sig != *b"8BIM" || record.is_none() {
            if sig == *b"8BIM" && matches!(&key, b"Patt" | b"Pat2" | b"Pat3" | b"Txt2" | b"FMsk") {
                s.omitted(
                    format!("文書タグ {}", String::from_utf8_lossy(&key)),
                    start,
                    length,
                )
            } else {
                s.preserve(
                    "TaggedBlock",
                    format!("未対応のタグ {}", String::from_utf8_lossy(&key)),
                    start,
                    length,
                )
            }
            continue;
        }
        let rec = record.as_deref_mut().unwrap();
        let duplicate = !seen.insert(key);
        match &key {
            b"lyid" => {
                if duplicate || size != 4 {
                    s.preserve("LayerIdentity", "lyid が重複・不正です", start, length)
                } else {
                    rec.layer.id = b.i32()?
                }
            }
            b"luni" => {
                if unicode {
                    s.preserve("UnicodeName", "luni が重複しています", start, length);
                    continue;
                }
                let n = b.u32()? as usize;
                check_budget(n <= s.limits.max_name_code_units, "Unicode名長の予算超過")?;
                rec.layer.name = b.utf16(n)?;
                if b.remaining() > 3 {
                    s.preserve("UnicodeNameTail", "未知のluni末尾", b.pos, b.remaining())
                } else {
                    b.zeros(b.remaining())?
                }
                if rec.layer.name.contains('\0') {
                    s.preserve("UnicodeNameNull", "luni 内のNUL", start, length)
                }
                unicode = true
            }
            b"iOpa" | b"clbl" | b"infx" | b"knko" | b"tsly" => {
                let (value, code) = match &key {
                    b"iOpa" => (255, "FillOpacity"),
                    b"clbl" => (1, "ClippedBlend"),
                    b"infx" => (0, "InteriorBlend"),
                    b"knko" => (0, "Knockout"),
                    _ => (1, "TransparencyShapes"),
                };
                if duplicate || b.data[b.pos..b.end] != [value, 0, 0, 0] {
                    s.preserve(
                        code,
                        format!("既定値以外の {}", String::from_utf8_lossy(&key)),
                        start,
                        length,
                    )
                }
            }
            b"lspf" => {
                if duplicate || size != 4 {
                    s.preserve("LayerLocks", "lspf が重複・不正です", start, length)
                } else {
                    rec.protection = b.u32()?
                }
            }
            b"lclr" => {
                if size != 8 {
                    s.preserve("SheetColor", "lclr の大きさが不正です", start, length)
                } else if b.u32()? != 0 || b.u32()? != 0 {
                    s.omitted("レイヤー色ラベル lclr", start, length)
                }
            }
            b"lnsr" | b"shmd" | b"fxrp" | b"lyvr" => s.omitted(
                format!("レイヤータグ {}", String::from_utf8_lossy(&key)),
                start,
                length,
            ),
            b"brst" => {
                if size != 0 {
                    s.preserve("ChannelRestrictions", "チャンネル合成制限", start, length)
                }
            }
            b"lsct" | b"lsdk" => {
                if rec.section != -1 || duplicate || !matches!(size, 4 | 12 | 16) {
                    rec.unknown_section = true;
                    s.preserve(
                        "SectionDivider",
                        "重複・未知のグループ区切り",
                        start,
                        length,
                    );
                    continue;
                }
                let kind = b.u32()?;
                if kind > 3 {
                    rec.unknown_section = true;
                    s.preserve("SectionDivider", "未知のグループ区切り種別", start, length);
                    continue;
                }
                rec.section = kind as i32;
                if size >= 12 {
                    check(b.key()? == *b"8BIM", "区切り合成シグネチャが不正です")?;
                    rec.section_key = Some(b.key()?)
                }
                if size == 16 {
                    rec.subtype = b.i32()?
                }
            }
            b"nvrt" | b"levl" | b"hue2" => {
                if rec.adjustment_seen {
                    s.preserve("Adjustment", "調整が重複しています", start, length);
                    continue;
                }
                rec.adjustment_seen = true;
                let a = match &key {
                    b"nvrt" => {
                        check(size == 0, "nvrt は空でなければなりません")?;
                        Some(Adjustment::Invert)
                    }
                    b"levl" => levels(b, s, start, length)?,
                    _ => hue(b, s, start, length)?,
                };
                if let Some(a) = a {
                    rec.layer.kind = LayerKind::Adjustment(a)
                }
            }
            b"SoCo" => {
                if rec.fill_seen || rec.adjustment_seen {
                    s.preserve(
                        "FillLayer",
                        "塗りつぶし・調整が重複しています",
                        start,
                        length,
                    );
                    continue;
                }
                rec.fill_seen = true;
                if let Some(rgb) = solid(b, s, start, length)? {
                    rec.layer.kind = LayerKind::SolidColor(rgb)
                }
            }
            _ => s.preserve(
                "TaggedBlock",
                format!("未対応のレイヤータグ {}", String::from_utf8_lossy(&key)),
                start,
                length,
            ),
        }
    }
    Ok(unicode)
}
fn levels(mut r: Reader, s: &mut State, start: usize, len: usize) -> Result<Option<Adjustment>> {
    check(r.remaining() >= 292, "levl が29レコードより短いです")?;
    if r.u16()? != 2 {
        s.preserve("Levels", "未知のlevl版", start, len);
        return Ok(None);
    }
    fn item(r: &mut Reader) -> Result<[u16; 5]> {
        Ok([r.u16()?, r.u16()?, r.u16()?, r.u16()?, r.u16()?])
    }
    let mut records = Vec::new();
    for _ in 0..29 {
        records.push(item(&mut r)?)
    }
    if r.remaining() >= 6 {
        let key = r.key()?;
        let v = r.u16()?;
        if key != *b"Lvls" || v != 3 {
            s.preserve("Levels", "未知のlevl拡張", start, len);
            return Ok(None);
        }
        let count = r.u16()? as usize;
        check(
            count >= 29 && (count - 29) * 10 <= r.remaining(),
            "Lvlsのレコード数が不正です",
        )?;
        for _ in 29..count {
            records.push(item(&mut r)?)
        }
    }
    if r.take(r.remaining())?.iter().any(|b| *b != 0) {
        s.preserve("Levels", "未知のlevl末尾", start, len);
        return Ok(None);
    }
    for (i, v) in records.iter().enumerate().skip(1) {
        if *v != [0, 255, 0, 255, 100] && !(i > 3 && *v == [0; 5]) {
            s.preserve(
                "Levels",
                "チャンネル別・追加チャンネルのレベル補正",
                start,
                len,
            );
            return Ok(None);
        }
    }
    let [ib, iw, ob, ow, g] = records[0];
    if ib > 253
        || !(2..=255).contains(&iw)
        || iw <= ib
        || ob > 255
        || ow > 255
        || !(10..=999).contains(&g)
    {
        s.preserve("Levels", "levl の値が対応範囲外です", start, len);
        return Ok(None);
    }
    Ok(Some(Adjustment::Levels {
        input_black: ib,
        input_white: iw,
        output_black: ob,
        output_white: ow,
        gamma: g,
    }))
}
pub(super) const HUE_RANGES: [[i16; 4]; 6] = [
    [315, 345, 15, 45],
    [15, 45, 75, 105],
    [75, 105, 135, 165],
    [135, 165, 195, 225],
    [195, 225, 255, 285],
    [255, 285, 315, 345],
];
fn hue(mut r: Reader, s: &mut State, start: usize, len: usize) -> Result<Option<Adjustment>> {
    check(r.remaining() >= 100, "hue2 が短すぎます")?;
    if r.u16()? != 2 {
        s.preserve("HueSaturation", "未知のhue2版", start, len);
        return Ok(None);
    }
    let colorize = r.u8()?;
    let padding = r.u8()?;
    let sliders = [r.i16()?, r.i16()?, r.i16()?];
    let hue = r.i16()?;
    let saturation = r.i16()?;
    let lightness = r.i16()?;
    let mut edits = false;
    let mut defaults = true;
    for range in HUE_RANGES {
        for v in range {
            defaults &= r.i16()? == v
        }
        for _ in 0..3 {
            edits |= r.i16()? != 0
        }
    }
    let tail = r.take(r.remaining())?;
    let known = padding == 0
        && if tail.len() == 36 {
            tail.as_chunks::<6>()
                .0
                .iter()
                .all(|b| b[2..] == [0, 100, 0, 50])
        } else {
            tail.len() <= 3 && tail.iter().all(|b| *b == 0)
        };
    if !known
        || colorize != 0
        || edits
        || !(-180..=180).contains(&hue)
        || !(-100..=100).contains(&saturation)
        || !(-100..=100).contains(&lightness)
    {
        s.preserve(
            "HueSaturation",
            "未知または対応範囲外のhue2・Colorize・色範囲編集",
            start,
            len,
        );
        return Ok(None);
    }
    if sliders != [0, 25, 0] && sliders != [0, 0, 0] {
        s.omitted("無効なColorizeのスライダー", start, len)
    }
    if !defaults {
        s.omitted("未編集の色範囲スライダー", start, len)
    }
    Ok(Some(Adjustment::HueSaturation {
        hue,
        saturation,
        lightness,
    }))
}
fn solid(mut r: Reader, s: &mut State, start: usize, len: usize) -> Result<Option<[u8; 3]>> {
    if r.u32()? != 16 {
        s.preserve("FillLayer", "未知のSoCo版", start, len);
        return Ok(None);
    }
    match super::descriptor::color(&mut r) {
        Ok(values) => {
            if values
                .iter()
                .any(|v| !v.is_finite() || *v < -1e-6 || *v > 255.0 + 1e-6)
            {
                s.preserve("FillLayer", "SoCo のRGBが範囲外です", start, len);
                return Ok(None);
            }
            let rgb = values.map(|v| v.round_ties_even() as u8);
            if values
                .iter()
                .zip(rgb)
                .any(|(v, b)| (*v - f64::from(b)).abs() > 1e-6)
            {
                s.omitted("SoCo の8bit未満の端数（丸め）", start, len)
            }
            Ok(Some(rgb))
        }
        Err(e) => {
            s.preserve("FillLayer", format!("未対応のSoCo記述子: {e}"), start, len);
            Ok(None)
        }
    }
}
