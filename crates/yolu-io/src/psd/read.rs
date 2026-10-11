use super::import::{self, CopyState};
use super::{binary::Reader, *};
use crate::{check, check_budget, Error, Result};
use std::collections::HashSet;
use std::io::Read;
use std::sync::atomic::AtomicBool;

pub(super) struct State<'a> {
    pub(super) limits: &'a Limits,
    /// 参照合成との照合（時間のかかる所）が見る取消の旗。
    pub(super) cancel: Option<&'a AtomicBool>,
    pub(super) notes: Vec<Diagnostic>,
    pub(super) unsupported: bool,
    pub(super) metadata: usize,
    pub(super) pixels: u64,
    pub(super) omitted: Vec<(String, usize, usize, usize)>,
    /// いま読んでいるタグ・合成キーの 4 文字（写しとしての取り込みが、理由の仕分けに使う）。
    pub(super) key: [u8; 4],
    /// 写しとしての取り込み（`import_copy`）のときだけ。原本を保つ読みでは None で、これまでと同じ動き。
    pub(super) copy: Option<CopyState>,
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
    pub(super) fn preserve(
        &mut self,
        code: &str,
        message: impl Into<String>,
        offset: usize,
        length: usize,
    ) {
        if self.copy.is_some() {
            // 写しとしての取り込み: 原本を保たないので「保つだけ」にはせず、理由の表（`import::sort_preserve`）で仕分ける
            return import::sort_preserve(self, code);
        }
        self.unsupported = true;
        self.note(code, message, offset, length)
    }
    /// 編集後の書き出しに含まれない情報（原本を保つ読みでの知らせ）。写しとしての取り込みでは何も知らせない
    /// （知らせたいものは `omit_as` で機能の名前つきで知らせる）。
    fn omitted(&mut self, what: impl Into<String>, offset: usize, length: usize) {
        if self.copy.is_some() {
            return;
        }
        let what = what.into();
        if let Some(x) = self.omitted.iter_mut().find(|x| x.0 == what) {
            x.3 += 1
        } else {
            self.omitted.push((what, offset, length, 1))
        }
    }
    /// 写しとしての取り込みで、見え方に効かない情報を持たないことを機能の名前つきで知らせる。原本を保つ読みでは `omitted` と同じ。
    fn omit_as(
        &mut self,
        feature: import::ImportFeature,
        detail: Option<import::ImportDetail>,
        what: impl Into<String>,
        offset: usize,
        length: usize,
    ) {
        if self.copy.is_some() {
            self.copy_note(feature, import::ImportAction::Ignored, detail);
        } else {
            self.omitted(what, offset, length)
        }
    }
    fn metadata(&mut self, n: usize) -> Result<()> {
        if self.copy.is_some() {
            // 写しとしての取り込みは、付加情報を持ち続けない（レイヤーごと・タグごとに読んで捨てる）ので、合計の予算は掛けない
            return Ok(());
        }
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
    read_cancellable(bytes, limits, None)
}
/// 取消の旗を、統合画像を参照合成と照らす間に見る読み込み（立っていたら `Error::Core(Cancelled)`。壊れた PSD の断りとは別）。
pub fn read_cancellable(
    bytes: &[u8],
    limits: &Limits,
    cancel: Option<&AtomicBool>,
) -> Result<ReadResult> {
    limits.validate()?;
    if bytes.len() > limits.max_source_bytes {
        return Ok(rejected(
            None,
            "SourceLimit",
            "PSD 原本の保持予算超過".into(),
        ));
    }
    read_owned(bytes.to_vec(), limits, cancel)
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
        read_owned(b, limits, None)
    }
}
fn read_owned(bytes: Vec<u8>, limits: &Limits, cancel: Option<&AtomicBool>) -> Result<ReadResult> {
    let mut s = State {
        limits,
        cancel,
        notes: Vec::new(),
        unsupported: false,
        metadata: 0,
        pixels: 0,
        omitted: Vec::new(),
        key: [0; 4],
        copy: None,
    };
    match parse(&bytes, &mut s) {
        // 取消は壊れた PSD ではない
        Err(e @ Error::Core(yolu_core::CoreError::Cancelled)) => Err(e),
        Err(e) => Ok(rejected(Some(bytes), "MalformedOrLimit", e.to_string())),
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
            Ok(ReadResult {
                mode: if s.unsupported {
                    CompatibilityMode::PreserveOnly
                } else {
                    CompatibilityMode::EditableRaster
                },
                document: if s.unsupported { None } else { doc },
                original: Some(bytes),
                diagnostics: s.notes,
            })
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
            let mut expected = super::composite::composite_cancellable(&doc, s.cancel)?;
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
pub(super) struct Record {
    pub(super) layer: Layer,
    pub(super) channels: Vec<(i16, usize)>,
    pub(super) section: i32,
    section_key: Option<[u8; 4]>,
    subtype: i32,
    unknown_section: bool,
    pub(super) adjustment_seen: bool,
    pub(super) fill_seen: bool,
    protection: u32,
    /// 明るさ・コントラストの 2 つの記録（`brit` と `CgEd`）。レイヤーのタグを読み終えてから突き合わせる。
    brightness: BrightnessRecords,
    /// 塗りの不透明度（`iOpa`。既定は 255）。写しとしての取り込みが不透明度に掛ける。
    pub(super) fill_opacity: u8,
    /// ベクターマスク（`vmsk`・`vsms`）を持つ。
    pub(super) vector_mask: bool,
}
#[derive(Default)]
struct BrightnessRecords {
    brit: Option<BritRecord>,
    cged: Option<CgedRecord>,
    /// 読めなかった記録の理由（旧式・新しい式のどちらも）。
    failed: Option<String>,
    /// 初めの記録の位置と長さ（診断の場所）。
    at: (usize, usize),
}
/// `brit`（旧式の記録）。
struct BritRecord {
    brightness: i16,
    contrast: i16,
    lab_only: bool,
}
/// `CgEd`（新しい式の記述子）。無い項目は既定値。
struct CgedRecord {
    version: i64,
    brightness: i64,
    contrast: i64,
    lab: bool,
    use_legacy: bool,
    auto: bool,
}
pub(super) fn record(r: &mut Reader, s: &mut State) -> Result<Record> {
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
        brightness: BrightnessRecords::default(),
        fill_opacity: 255,
        vector_mask: false,
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
    // 写しとしての取り込みは、定義とチャンネルが食い違うマスクを断らず、読める側だけ使う（呼び手が整える）
    if s.copy.is_none() {
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
    resolve_brightness_contrast(&mut rec, s);
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
            s.omit_as(
                import::ImportFeature::LayerLockBits,
                None,
                "lspf の未対応ロックビット",
                offset,
                0,
            )
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
        s.key = mode;
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
                s.omit_as(
                    import::ImportFeature::CollapsedGroup,
                    None,
                    "閉じたグループ",
                    offset,
                    0,
                )
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
    let mut color = r.u8()?;
    if s.copy.is_some() && !matches!(color, 0 | 255) {
        // 写しとしての取り込み: 0・255 以外の既定値は、近いほうへ寄せる（矩形の外の見え方が変わるので知らせる）
        color = if color >= 128 { 255 } else { 0 };
        s.copy_note(
            import::ImportFeature::MaskDefault,
            import::ImportAction::Changed,
            None,
        )
    }
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
pub(super) fn decode(
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
        // ZIP（2）と予測つき ZIP（3）。写しとしての取り込みだけが読む（原本を保つ読みは、これまでどおり未対応として原本を保つ）
        2 | 3 if s.copy.is_some() => {
            let (w, h) = (w as usize, h as usize);
            if w > 0 && h > 0 {
                let mut plane = vec![0u8; w * h];
                flate2::read::ZlibDecoder::new(&r.data[r.pos..r.end])
                    .read_exact(&mut plane)
                    .map_err(|_| Error::InvalidData("ZIP チャンネルを展開できません".into()))?;
                if compression == 3 {
                    // 予測: 行の中で、左の画素との差で持っている
                    for row in plane.chunks_exact_mut(w) {
                        for x in 1..w {
                            row[x] = row[x].wrapping_add(row[x - 1])
                        }
                    }
                }
                for (i, v) in plane.iter().enumerate() {
                    out[i * stride + component] = *v
                }
            }
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

/// 行を並べて確かめる、RLE のチャンネルの圧縮した行の合計の下限（これより小さければ 1 本で確かめる）。
const PARALLEL_CHECK_BYTES: usize = 1 << 20;

/// 無圧縮か RLE のチャンネルを、行ごとに並べて復号する形（`decode` と同じ所へ同じ値を書き、同じ誤りを返す）。RLE の行は互いに独立
/// （表に行ごとの長さがある）なので、行の区間を先に切り出し（`decode` が順に切り出すのと同じ確かめ）、行の展開をワーカーへ分ける。
/// 誤りは、`decode` が先に出会うもの: 展開に失敗した一番上の行、無ければ行を切り出せなかった所・末尾の余り。
pub(super) struct ChannelRows<'a> {
    reader: Reader<'a>,
    width: usize,
    height: usize,
    /// RLE の行の区間（読み手の中の位置）。無圧縮なら空。
    rows: Vec<(usize, usize)>,
    raw: bool,
    /// 行の区間を切り出したあとの誤り（切り出せなかった行・末尾の余り）。それより上の行の展開の誤りが先。
    after: Option<Error>,
}

impl<'a> ChannelRows<'a> {
    /// 圧縮の種類と行の区間を読む。無圧縮・RLE でなければ（ZIP・未対応）、幅か高さが 0 なら None（`decode` で 1 本ずつ）。
    pub(super) fn new(mut r: Reader<'a>, w: u32, h: u32) -> Result<Option<ChannelRows<'a>>> {
        let compression = r.u16()?;
        let (w, h) = (w as usize, h as usize);
        if w == 0 || h == 0 || compression > 1 {
            return Ok(None);
        }
        let mut rows = Vec::new();
        let mut after = None;
        if compression == 0 {
            check(r.remaining() == w * h, "raw チャンネル長が矩形と不一致です")?;
        } else {
            let mut table = r.slice(h * 2)?;
            rows.reserve(h);
            for _ in 0..h {
                let n = table.u16()? as usize;
                match r.slice(n) {
                    Ok(row) => rows.push((row.pos, row.end)),
                    Err(e) => {
                        after = Some(e);
                        break;
                    }
                }
            }
            if after.is_none() {
                after = check(r.remaining() == 0, "RLE チャンネルに余分なデータ").err();
            }
        }
        Ok(Some(ChannelRows {
            reader: r,
            width: w,
            height: h,
            rows,
            raw: compression == 0,
            after,
        }))
    }

    /// `decode` と同じ誤りを返すが、画素はどこにも書かない（無圧縮は `new` で長さを確かめ済み。RLE の行はワーカーごとの 1 行の場所へ展開する）。
    /// RLE の圧縮した行の合計が `parallel_from` より小さければ、1 本で確かめる。
    fn check(self, parallel_from: usize) -> Result<()> {
        use rayon::prelude::*;
        if self.raw {
            return Ok(());
        }
        let (w, r) = (self.width, &self.reader);
        let unpack = |row: &mut Vec<u8>, (y, &(pos, end)): (usize, &(usize, usize))| {
            let line = Reader {
                data: r.data,
                pos,
                end,
                lenient: r.lenient,
            };
            packbits(line, w, row, 0, 1).err().map(|e| (y, e))
        };
        let packed = match (self.rows.first(), self.rows.last()) {
            (Some(first), Some(last)) => last.1 - first.0,
            _ => 0,
        };
        // 小さなチャンネルは 1 本で（展開は書かずに読むだけで速く、並べる手間と、ほかの仕事で混んだ機械でワーカーを待つ時間のほうが大きい）
        let first = if packed < parallel_from {
            let mut row = vec![0u8; w];
            self.rows
                .iter()
                .enumerate()
                .find_map(|item| unpack(&mut row, item))
        } else {
            self.rows
                .par_iter()
                .enumerate()
                .map_init(|| vec![0u8; w], unpack)
                .flatten()
                .min_by_key(|(y, _)| *y)
        };
        match (first, self.after) {
            (Some((_, e)), _) | (None, Some(e)) => Err(e),
            (None, None) => Ok(()),
        }
    }

    /// 行ごとに並べて、out（幅 × stride のバイトの行が上から並ぶ面）の component へ展開する。
    pub(super) fn decode(self, out: &mut [u8], component: usize, stride: usize) -> Result<()> {
        use rayon::prelude::*;
        let (w, r) = (self.width, &self.reader);
        if self.raw {
            let start = r.pos;
            out.par_chunks_mut(w * stride)
                .take(self.height)
                .enumerate()
                .for_each(|(y, row)| {
                    let src = &r.data[start + y * w..start + (y + 1) * w];
                    for (x, v) in src.iter().enumerate() {
                        row[component + x * stride] = *v;
                    }
                });
            return Ok(());
        }
        let first = out
            .par_chunks_mut(w * stride)
            .zip(self.rows.par_iter())
            .enumerate()
            .filter_map(|(y, (row, &(pos, end)))| {
                let line = Reader {
                    data: r.data,
                    pos,
                    end,
                    lenient: r.lenient,
                };
                packbits(line, w, row, component, stride)
                    .err()
                    .map(|e| (y, e))
            })
            .min_by_key(|(y, _)| *y);
        match (first, self.after) {
            (Some((_, e)), _) | (None, Some(e)) => Err(e),
            (None, None) => Ok(()),
        }
    }
}

/// `decode_rows` の、復号できるかだけを見る形（誤り・知らせは同じ）。無圧縮・RLE は画素を書かずに確かめる（RLE の行はワーカーごとの
/// 1 行の場所へ展開する）。ほかの圧縮は `plane` を面の大きさにして `decode` で復号する。
pub(super) fn check_rows(
    r: Reader,
    w: u32,
    h: u32,
    plane: &mut Vec<u8>,
    s: &mut State,
) -> Result<()> {
    check_rows_from(r, w, h, plane, s, PARALLEL_CHECK_BYTES)
}
/// `check_rows` の、行を並べる大きさの下限を渡す形（試験が 1 本の道と並べる道の両方を通す）。
fn check_rows_from(
    r: Reader,
    w: u32,
    h: u32,
    plane: &mut Vec<u8>,
    s: &mut State,
    parallel_from: usize,
) -> Result<()> {
    match ChannelRows::new(r.clone(), w, h)? {
        Some(rows) => rows.check(parallel_from),
        None => {
            // 中身は使わないので、前のチャンネルの中身は消さずに大きさだけ合わせる
            plane.resize(w as usize * h as usize, 0);
            decode(r, w, h, plane, 0, 1, s)
        }
    }
}
/// `decode` の、無圧縮・RLE のチャンネルを行ごとに並べて復号する形（ほかの圧縮は `decode` で 1 本ずつ）。書く値・誤り・知らせは `decode` と同じ。
pub(super) fn decode_rows(
    r: Reader,
    w: u32,
    h: u32,
    out: &mut [u8],
    component: usize,
    stride: usize,
    s: &mut State,
) -> Result<()> {
    match ChannelRows::new(r.clone(), w, h)? {
        Some(rows) => rows.decode(out, component, stride),
        None => decode(r, w, h, out, component, stride, s),
    }
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
        // 1 面へ（stride 1）はまとめて写す・埋める
        let at = offset + x * stride;
        if control >= 0 {
            let src = r.take(n)?;
            if stride == 1 {
                out[at..at + n].copy_from_slice(src);
                x += n;
            } else {
                for b in src {
                    out[offset + x * stride] = *b;
                    x += 1
                }
            }
        } else {
            let b = r.u8()?;
            if stride == 1 {
                out[at..at + n].fill(b);
                x += n;
            } else {
                for _ in 0..n {
                    out[offset + x * stride] = b;
                    x += 1
                }
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
pub(super) fn icc(r: &Reader) -> Option<String> {
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
        s.key = key;
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
                    format!("ドキュメントタグ {}", String::from_utf8_lossy(&key)),
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
                // 写しとしての取り込みは、塗りの不透明度を不透明度に掛ける（原本を保つ読みでは使わない）
                if s.copy.is_some() && &key == b"iOpa" && !duplicate && size == 4 {
                    rec.fill_opacity = b.data[b.pos]
                }
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
                    s.omit_as(
                        import::ImportFeature::LayerColorLabel,
                        None,
                        "レイヤー色ラベル lclr",
                        start,
                        length,
                    )
                }
            }
            b"lnsr" | b"shmd" | b"fxrp" | b"lyvr" => s.omit_as(
                import::ImportFeature::LayerMetadata,
                Some(import::ImportDetail::Key(key)),
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
            b"brit" | b"CgEd" => {
                if duplicate {
                    s.preserve("Adjustment", "調整が重複しています", start, length);
                    continue;
                }
                if rec.brightness.at == (0, 0) {
                    rec.brightness.at = (start, length)
                }
                // 突き合わせはレイヤーのタグを読み終えてから（`resolve_brightness_contrast`）
                if &key == b"brit" {
                    match brit_record(b)? {
                        Ok(v) => rec.brightness.brit = Some(v),
                        Err(why) => rec.brightness.failed = Some(why),
                    }
                } else {
                    match cged_record(b)? {
                        Ok(v) => rec.brightness.cged = Some(v),
                        Err(why) => rec.brightness.failed = Some(why),
                    }
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
            // 色調補正の 5 種（thrs・post・blnc・curv・grdm。brit/CgEd は下で突き合わせる）。表せない中身は、これまでと同じ未対応のタグとして原本を保つ（調整とは数えない）
            b"thrs" | b"post" | b"blnc" | b"curv" | b"grdm" => {
                if rec.adjustment_seen {
                    s.preserve("Adjustment", "調整が重複しています", start, length);
                    continue;
                }
                let parsed = match &key {
                    b"thrs" => level_record(b, true)?,
                    b"post" => level_record(b, false)?,
                    b"blnc" => color_balance(b)?,
                    b"curv" => curves(b)?,
                    _ => gradient_map(b, s, start, length)?,
                };
                match parsed {
                    Ok(a) => {
                        rec.adjustment_seen = true;
                        rec.layer.kind = LayerKind::Adjustment(a)
                    }
                    Err(why) => s.preserve(
                        "TaggedBlock",
                        format!(
                            "未対応のレイヤータグ {}（{why}）",
                            String::from_utf8_lossy(&key)
                        ),
                        start,
                        length,
                    ),
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
            b"vmsk" | b"vsms" if s.copy.is_some() => {
                rec.vector_mask = true;
                s.preserve(
                    "TaggedBlock",
                    format!("未対応のレイヤータグ {}", String::from_utf8_lossy(&key)),
                    start,
                    length,
                )
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
/// 色調補正の記録を読んだ結果: 調整にできたか、できないなら理由（できないものは、これまでと同じ「未対応のレイヤータグ」として原本を保つ）。
type Parsed<T = Adjustment> = Result<std::result::Result<T, String>>;
fn unsupported<T>(why: impl Into<String>) -> Parsed<T> {
    Ok(Err(why.into()))
}
/// しきい値（`thrs`）・ポスタリゼーション（`post`）: 2 バイトの値と 2 バイトの余白。
fn level_record(mut r: Reader, threshold: bool) -> Parsed {
    if !matches!(r.remaining(), 2 | 4) {
        return unsupported("大きさが不正です");
    }
    let v = r.u16()?;
    if r.take(r.remaining())?.iter().any(|b| *b != 0) {
        return unsupported("未知の末尾");
    }
    let range = if threshold { 1..=255 } else { 2..=255 };
    if !range.contains(&v) {
        return unsupported("値が対応範囲外です");
    }
    Ok(Ok(if threshold {
        Adjustment::Threshold { level: v }
    } else {
        Adjustment::Posterize { levels: v }
    }))
}
/// 明るさ・コントラストの旧式の記録（`brit`）: 明るさ・コントラスト・平均値（各 2 バイト、符号つき）、Lab だけの 1 バイト、余白 1 バイト。
fn brit_record(mut r: Reader) -> Parsed<BritRecord> {
    if !matches!(r.remaining(), 7 | 8) {
        return unsupported("brit の大きさが不正です");
    }
    let brightness = r.i16()?;
    let contrast = r.i16()?;
    let _mean = r.i16()?;
    let lab_only = r.u8()?;
    if r.take(r.remaining())?.iter().any(|b| *b != 0) || lab_only > 1 {
        return unsupported("未知の brit 末尾・Lab");
    }
    Ok(Ok(BritRecord {
        brightness,
        contrast,
        lab_only: lab_only == 1,
    }))
}
/// 明るさ・コントラストの新しい式の記録（`CgEd`: 版 16 と、明るさ `Brgh`・コントラスト `Cntr`・平均値 `means`・Lab `Lab `・旧式 `useLegacy`・
/// 自動 `auto`・版 `Vrsn` だけの平らな記述子）。ほかの項目や入れ子を持つものは、このツールの知らない内容なので対応しない。
fn cged_record(mut r: Reader) -> Parsed<CgedRecord> {
    use super::descriptor::Scalar;
    if r.remaining() < 4 || r.u32()? != 16 {
        return unsupported("未知のCgEd版");
    }
    let items = match super::descriptor::flat(&mut r) {
        Ok(Some(items)) => items,
        Ok(None) => return unsupported("未対応のCgEd記述子（入れ子・文字列）"),
        Err(e) => return unsupported(format!("未対応のCgEd記述子: {e}")),
    };
    let mut record = CgedRecord {
        version: 1,
        brightness: 0,
        contrast: 0,
        lab: false,
        use_legacy: false,
        auto: false,
    };
    let mut seen = Vec::new();
    for (key, value) in items {
        let number = |v: Scalar| match v {
            Scalar::Number(n) if n.fract() == 0.0 && n.abs() < 1e9 => Some(n as i64),
            _ => None,
        };
        let flag = |v: Scalar| match v {
            Scalar::Bool(b) => Some(b),
            _ => None,
        };
        let ok = !seen.contains(&key)
            && match key.as_slice() {
                b"Vrsn" => number(value).map(|v| record.version = v).is_some(),
                b"Brgh" => number(value).map(|v| record.brightness = v).is_some(),
                b"Cntr" => number(value).map(|v| record.contrast = v).is_some(),
                b"means" => number(value).is_some(),
                b"Lab " => flag(value).map(|v| record.lab = v).is_some(),
                b"useLegacy" => flag(value).map(|v| record.use_legacy = v).is_some(),
                b"auto" => flag(value).map(|v| record.auto = v).is_some(),
                _ => false,
            };
        if !ok {
            return unsupported(format!(
                "未知・重複・型違いのCgEdの項目 {}",
                String::from_utf8_lossy(&key)
            ));
        }
        seen.push(key);
    }
    Ok(Ok(record))
}
/// レイヤーのタグを読み終えてから、明るさ・コントラストの `brit` と `CgEd` を突き合わせて調整にする。編集できるのは、新しい式（`CgEd` があり、
/// 旧式・自動・Lab でない）で、`brit` があれば同じ値のものだけ。`brit` だけ（旧式）・旧式の `CgEd`・範囲外・読めない記録は、このツールの式で
/// 表せないので、ほかの対応しないタグと同じく原本を保つ。
fn resolve_brightness_contrast(rec: &mut Record, s: &mut State) {
    let BrightnessRecords {
        brit,
        cged,
        failed,
        at: (start, length),
    } = std::mem::take(&mut rec.brightness);
    if brit.is_none() && cged.is_none() && failed.is_none() {
        return;
    }
    let mut refuse = |why: &str| {
        // 写しとしての取り込みが、このレイヤーを明るさ・コントラストの調整として仕分けられるように
        s.key = *b"brit";
        s.preserve(
            "TaggedBlock",
            format!("未対応のレイヤータグ brit/CgEd（{why}）"),
            start,
            length,
        )
    };
    if let Some(why) = failed {
        return refuse(&why);
    }
    if rec.adjustment_seen {
        return refuse("調整が重複しています");
    }
    let Some(cged) = cged else {
        return refuse("旧式の brit だけの明るさ・コントラスト（新しい式の CgEd が無い）");
    };
    let bad = cged.version != 1
        || cged.lab
        || cged.use_legacy
        || cged.auto
        || brit.as_ref().is_some_and(|b| {
            b.lab_only
                || i64::from(b.brightness) != cged.brightness
                || i64::from(b.contrast) != cged.contrast
        });
    // コントラストの下限は −50（このツールの範囲。Photoshop は −100〜100）
    if bad || !(-150..=150).contains(&cged.brightness) || !(-50..=100).contains(&cged.contrast) {
        return refuse("旧式・自動・Lab・範囲外・brit と CgEd の食い違い");
    }
    s.omitted(
        "明るさ・コントラストの平均値（旧式の式の入力。このツールの式は使わない）",
        start,
        length,
    );
    rec.adjustment_seen = true;
    rec.layer.kind = LayerKind::Adjustment(Adjustment::BrightnessContrast {
        brightness: cged.brightness as i16,
        contrast: cged.contrast as i16,
    });
}
/// カラーバランス（`blnc`）: シャドウ・中間・ハイライトの各 3 つ（シアン/レッド・マゼンタ/グリーン・イエロー/ブルー、各 2 バイト符号つき）、
/// 輝度を保つ 1 バイト、余白 1 バイト。
fn color_balance(mut r: Reader) -> Parsed {
    if !matches!(r.remaining(), 19 | 20) {
        return unsupported("大きさが不正です");
    }
    let mut ranges = [[0i16; 3]; 3];
    for range in &mut ranges {
        for v in range.iter_mut() {
            *v = r.i16()?;
        }
    }
    let preserve = r.u8()?;
    if r.take(r.remaining())?.iter().any(|b| *b != 0) || preserve > 1 {
        return unsupported("未知の末尾・輝度の印");
    }
    if ranges.iter().flatten().any(|v| !(-100..=100).contains(v)) {
        return unsupported("値が対応範囲外です");
    }
    Ok(Ok(Adjustment::ColorBalance {
        shadows: ranges[0],
        midtones: ranges[1],
        highlights: ranges[2],
        preserve_luminosity: preserve == 1,
    }))
}
/// トーンカーブ（`curv`）: 0（画素の表ではない）・版 1・曲線のある チャンネルのビット（0:RGB 全体・1:R・2:G・3:B）、
/// 各曲線は点の数（2〜19）と点（出力・入力の 2 バイトずつ）。版 1 のあとに続く拡張（`Crv `）は、基本の曲線と同じ点を持つときだけ受ける。
fn curves(mut r: Reader) -> Parsed {
    if r.remaining() < 7 {
        return unsupported("短すぎます");
    }
    let is_map = r.u8()?;
    let version = r.u16()?;
    let channels = r.u32()?;
    if is_map != 0 || version != 1 || channels & !0xf != 0 || channels == 0 {
        return unsupported("表の形式・版・チャンネルが対応しません");
    }
    let mut curves: [Vec<[u8; 2]>; 4] = Default::default();
    for (i, curve) in curves.iter_mut().enumerate() {
        if channels & (1 << i) == 0 {
            *curve = vec![[0, 0], [255, 255]];
            continue;
        }
        if r.remaining() < 2 {
            return unsupported("点の数が途切れています");
        }
        let n = r.u16()? as usize;
        if !(2..=19).contains(&n) || r.remaining() < n * 4 {
            return unsupported("点の数が不正か点が途切れています");
        }
        for _ in 0..n {
            let (output, input) = (r.u16()?, r.u16()?);
            if output > 255 || input > 255 {
                return unsupported("点が 0〜255 の外です");
            }
            curve.push([input as u8, output as u8]);
        }
    }
    // 拡張（`Crv `）: 基本と同じ点なら受け、違えば対応しない
    let rest = r.take(r.remaining())?;
    if rest.iter().any(|b| *b != 0) && !curves_extra_matches(rest, channels, &curves) {
        return unsupported("拡張が基本の曲線と違います");
    }
    for curve in &curves {
        if !curve.windows(2).all(|w| w[0][0] < w[1][0]) {
            return unsupported("入力が昇順でありません");
        }
        // PSD として正しくても、このツールが表せる曲線（core の `Curve`: 点は 16 まで・両端の入力は 0 と 255・隣の点は 6 刻み以上）でなければ
        // 編集できるレイヤーにしない（Photoshop の「自動」は両端を動かすので、よくある形）。原本を保つ
        let points = curve
            .iter()
            .map(|p| yolu_core::curve::CurvePoint {
                x: f64::from(p[0]) / 255.0,
                y: f64::from(p[1]) / 255.0,
            })
            .collect();
        if yolu_core::curve::Curve::new(points).is_err() {
            return unsupported(
                "このツールが表せない曲線です（点は 16 まで・両端の入力は 0 と 255・隣の点は 6 刻み以上）",
            );
        }
    }
    let [composite, red, green, blue] = curves;
    Ok(Ok(Adjustment::ToneCurve {
        composite,
        red,
        green,
        blue,
    }))
}
/// `curv` の末尾の拡張（`Crv `・版 3 か 4・項目数、項目は チャンネル番号・点の数・点）が、基本の曲線と同じ点か。余白（0）は許す。
fn curves_extra_matches(rest: &[u8], channels: u32, base: &[Vec<[u8; 2]>; 4]) -> bool {
    let mut r = Reader::new(rest);
    let parse = |r: &mut Reader| -> Result<bool> {
        if r.key()? != *b"Crv " {
            return Ok(false);
        }
        let version = r.u16()?;
        let count = r.u32()? as usize;
        if !matches!(version, 3 | 4) || count > 4 {
            return Ok(false);
        }
        for _ in 0..count {
            let channel = r.u16()? as usize;
            let n = r.u16()? as usize;
            if channel > 3 || channels & (1 << channel) == 0 || n * 4 > r.remaining() {
                return Ok(false);
            }
            let mut points = Vec::new();
            for _ in 0..n {
                let (output, input) = (r.u16()?, r.u16()?);
                if output > 255 || input > 255 {
                    return Ok(false);
                }
                points.push([input as u8, output as u8]);
            }
            if points != base[channel] {
                return Ok(false);
            }
        }
        Ok(r.remaining() < 4 && r.take(r.remaining())?.iter().all(|b| *b == 0))
    };
    parse(&mut r).unwrap_or(false)
}
/// グラデーションマップ（`grdm`。版 1）: 逆向き・ディザ・名前・色の分岐点（位置・中点・モード・16 bit の色・余白）・不透明度の分岐点
/// （位置・中点・不透明度）・補間と乱数などの欄。ディザ・なめらかさが 100% でないもの・ノイズ型・RGB 以外の色・端の中点などは対応しない。
fn gradient_map(mut r: Reader, s: &mut State, start: usize, len: usize) -> Parsed {
    if r.remaining() < 10 {
        return unsupported("短すぎます");
    }
    let version = r.u16()?;
    let reverse = r.u8()?;
    let dither = r.u8()?;
    let name_len = r.u32()? as usize;
    if name_len > s.limits.max_name_code_units || r.remaining() < name_len * 2 + 2 {
        return unsupported("名前の長さが不正です");
    }
    let name = r.utf16(name_len)?;
    if version != 1 || reverse > 1 || dither != 0 {
        return unsupported("未知の版・ディザ・逆向きの印");
    }
    let count = r.u16()? as usize;
    if !(1..=256).contains(&count) || r.remaining() < count * 20 + 2 {
        return unsupported("色の分岐点の数が不正か途切れています");
    }
    let mut colors = Vec::new();
    let mut bad = false;
    let mut fractions = false;
    for _ in 0..count {
        let location = r.u32()?;
        let midpoint = r.u32()?;
        let mode = r.u16()?;
        let mut rgb = [0u8; 3];
        let components = [r.u16()?, r.u16()?, r.u16()?, r.u16()?];
        let pad = r.u16()?;
        for (out, v) in rgb.iter_mut().zip(components) {
            let byte = (u32::from(v) + 128) / 257;
            fractions |= byte * 257 != u32::from(v);
            *out = byte as u8;
        }
        bad |= location > 4096
            || !(1..=99).contains(&midpoint)
            || mode != 0
            || components[3] != 0
            || pad != 0;
        colors.push(GradientColorStop {
            location: location.min(4096) as u16,
            midpoint: midpoint.min(100) as u8,
            rgb,
        });
    }
    let count = r.u16()? as usize;
    if !(1..=256).contains(&count) || r.remaining() < count * 10 + 8 + 32 {
        return unsupported("不透明度の分岐点の数が不正か途切れています");
    }
    let mut opacities = Vec::new();
    for _ in 0..count {
        let location = r.u32()?;
        let midpoint = r.u32()?;
        let opacity = r.u16()?;
        bad |= location > 4096 || !(1..=99).contains(&midpoint) || opacity > 255;
        opacities.push(GradientOpacityStop {
            location: location.min(4096) as u16,
            midpoint: midpoint.min(100) as u8,
            opacity: opacity.min(255) as u8,
        });
    }
    // 補間（拡張 2・なめらかさ 4096 = 100%・長さ 32・モード 0 = 色の分岐点のグラデーション）。乱数・粗さ・色モデル・範囲はノイズ型の欄
    let (expansion, smoothness, length, mode) = (r.u16()?, r.u16()?, r.u16()?, r.u16()?);
    bad |= expansion != 2 || smoothness != 4096 || length != 32 || mode != 0;
    let seed = r.u32()?;
    let show_transparency = r.u16()?;
    let use_vector_color = r.u16()?;
    let roughness = r.u32()?;
    let color_model = r.u16()?;
    let ranges_differ = r.take(16)?.iter().any(|b| *b != 0);
    let dummy = r.u16()?;
    bad |= use_vector_color != 0;
    // ノイズ型の欄（乱数・透明の表示・粗さ・色モデル・色の範囲）は、書き戻すときの決まった値（`write.rs` の grdm）へ置き換わる。
    // 値が違うなら、黙って変えず「編集後の書き出しには含まれない」と知らせる
    let noise_fields_differ = seed != 0
        || show_transparency != 1
        || roughness != 0
        || color_model != 3
        || ranges_differ
        || dummy != 0;
    bad |= r.take(r.remaining())?.iter().any(|b| *b != 0);
    bad |= !(2..=32).contains(&colors.len())
        || !(2..=32).contains(&opacities.len())
        || !colors.windows(2).all(|w| w[0].location < w[1].location)
        || !opacities.windows(2).all(|w| w[0].location < w[1].location);
    if bad {
        return unsupported("ノイズ・補間・中点・色モード・分岐点の数と並びが対応しません");
    }
    if fractions {
        s.omitted("grdm の色の 8bit 未満の端数（丸め）", start, len)
    }
    if noise_fields_differ {
        s.omitted(
            "grdm のノイズ型の欄（乱数・透明の表示・粗さ・色モデル・色の範囲）",
            start,
            len,
        )
    }
    if !name.is_empty() {
        s.omitted("グラデーションの名前", start, len)
    }
    Ok(Ok(Adjustment::GradientMap {
        reverse: reverse == 1,
        colors,
        opacities,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn state(limits: &Limits) -> State<'_> {
        State {
            limits,
            cancel: None,
            notes: Vec::new(),
            unsupported: false,
            metadata: 0,
            pixels: 0,
            omitted: Vec::new(),
            key: [0; 4],
            copy: None,
        }
    }
    fn record(brit: Option<(i16, i16, bool)>, cged: Option<CgedRecord>) -> Record {
        Record {
            layer: Layer::default(),
            channels: Vec::new(),
            section: -1,
            section_key: None,
            subtype: 0,
            unknown_section: false,
            adjustment_seen: false,
            fill_seen: false,
            protection: 0,
            fill_opacity: 255,
            vector_mask: false,
            brightness: BrightnessRecords {
                brit: brit.map(|(brightness, contrast, lab_only)| BritRecord {
                    brightness,
                    contrast,
                    lab_only,
                }),
                cged,
                failed: None,
                at: (10, 20),
            },
        }
    }
    fn cged(brightness: i64, contrast: i64) -> CgedRecord {
        CgedRecord {
            version: 1,
            brightness,
            contrast,
            lab: false,
            use_legacy: false,
            auto: false,
        }
    }
    /// 解決した結果: 調整にできたか（できなければ保護の診断が付く）。
    fn resolved(mut rec: Record) -> (Option<Adjustment>, bool) {
        let limits = Limits::default();
        let mut s = state(&limits);
        resolve_brightness_contrast(&mut rec, &mut s);
        match rec.layer.kind {
            LayerKind::Adjustment(a) => (Some(a), s.unsupported),
            _ => (None, s.unsupported),
        }
    }

    #[test]
    fn brightness_and_contrast_need_the_new_record_and_agree_with_the_old_one() {
        // 新しい式の記録だけ（Photoshop の新しい書き方）・両方が同じ値（このツールの書き方）は調整になる
        for brit in [None, Some((-37, 81, false))] {
            let (a, unsupported) = resolved(record(brit, Some(cged(-37, 81))));
            assert_eq!(
                a,
                Some(Adjustment::BrightnessContrast {
                    brightness: -37,
                    contrast: 81
                })
            );
            assert!(!unsupported);
        }
        // 旧式の brit だけ・どちらも無い・読めなかった記録がある
        assert_eq!(resolved(record(Some((10, 10, false)), None)), (None, true));
        assert_eq!(resolved(record(None, None)), (None, false));
        let mut broken = record(None, Some(cged(1, 1)));
        broken.brightness.failed = Some("壊れている".into());
        assert_eq!(resolved(broken), (None, true));
        // 食い違い・Lab・旧式・自動・版・範囲外は保護する
        assert_eq!(
            resolved(record(Some((10, 10, false)), Some(cged(11, 10)))),
            (None, true)
        );
        assert_eq!(
            resolved(record(Some((10, 10, true)), Some(cged(10, 10)))),
            (None, true)
        );
        for edit in [
            |c: &mut CgedRecord| c.lab = true,
            |c: &mut CgedRecord| c.use_legacy = true,
            |c: &mut CgedRecord| c.auto = true,
            |c: &mut CgedRecord| c.version = 2,
        ] {
            let mut c = cged(5, 5);
            edit(&mut c);
            assert_eq!(resolved(record(None, Some(c))), (None, true));
        }
        for (b, c) in [(151, 0), (-151, 0), (0, 101), (0, -51)] {
            assert_eq!(
                resolved(record(None, Some(cged(b, c)))),
                (None, true),
                "{b} {c}"
            );
        }
        for (b, c) in [(150, 100), (-150, -50)] {
            assert!(
                resolved(record(None, Some(cged(b, c)))).0.is_some(),
                "{b} {c}"
            );
        }
    }

    #[test]
    fn the_curves_extra_must_repeat_the_base_curves() {
        let base: [Vec<[u8; 2]>; 4] = [
            vec![[0, 0], [100, 120], [255, 255]],
            vec![[0, 0], [255, 255]],
            vec![[0, 0], [255, 255]],
            vec![[0, 0], [255, 255]],
        ];
        // 項目は チャンネル番号・点の数・点（出力・入力）
        let extra = |version: u16, items: &[(u16, &[[u8; 2]])]| {
            let mut b = Vec::new();
            b.extend(b"Crv ");
            b.extend(version.to_be_bytes());
            b.extend((items.len() as u32).to_be_bytes());
            for (channel, points) in items {
                b.extend(channel.to_be_bytes());
                b.extend((points.len() as u16).to_be_bytes());
                for [input, output] in *points {
                    b.extend(u16::from(*output).to_be_bytes());
                    b.extend(u16::from(*input).to_be_bytes());
                }
            }
            b
        };
        let same = extra(4, &[(0, &base[0]), (1, &base[1])]);
        assert!(curves_extra_matches(&same, 0xf, &base));
        // 末尾の余白は許す
        let mut padded = same.clone();
        padded.extend([0, 0]);
        assert!(curves_extra_matches(&padded, 0xf, &base));
        // 点が違う・版が違う・チャンネルが無い・壊れている
        let other = extra(4, &[(0, &[[0, 0], [100, 121], [255, 255]])]);
        assert!(!curves_extra_matches(&other, 0xf, &base));
        assert!(!curves_extra_matches(
            &extra(5, &[(0, &base[0])]),
            0xf,
            &base
        ));
        assert!(!curves_extra_matches(
            &extra(4, &[(0, &base[0])]),
            0xe,
            &base
        ));
        assert!(!curves_extra_matches(&same[..same.len() - 3], 0xf, &base));
        assert!(!curves_extra_matches(b"XXXXrest", 0xf, &base));
    }

    /// PackBits で 1 行を詰める（3 つ以上続く値は繰り返し、ほかは直書き。どちらも 128 まで）。
    fn pack(row: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < row.len() {
            let mut run = 1;
            while i + run < row.len() && row[i + run] == row[i] && run < 128 {
                run += 1;
            }
            if run >= 3 {
                out.push((257 - run) as u8);
                out.push(row[i]);
                i += run;
                continue;
            }
            let start = i;
            while i < row.len() && i - start < 128 {
                if i + 2 < row.len() && row[i] == row[i + 1] && row[i] == row[i + 2] {
                    break;
                }
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend_from_slice(&row[start..i]);
        }
        out
    }

    /// RLE のチャンネル（圧縮の番号・行の長さの表・行）。rows は詰めた行。
    fn rle(rows: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0, 1];
        for r in rows {
            out.extend((r.len() as u16).to_be_bytes());
        }
        for r in rows {
            out.extend(r);
        }
        out
    }

    fn packed_rows(w: usize, h: usize) -> Vec<Vec<u8>> {
        let mut seed = 0x9e37_79b9u32;
        (0..h)
            .map(|y| {
                let row: Vec<u8> = (0..w)
                    .map(|x| {
                        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                        if (x / 5 + y) % 3 == 0 {
                            (y * 3) as u8
                        } else {
                            (seed >> 24) as u8
                        }
                    })
                    .collect();
                let mut p = pack(&row);
                if y % 4 == 1 {
                    // 何もしない印（-128）も読み飛ばす
                    p.insert(0, 0x80);
                }
                p
            })
            .collect()
    }

    /// 行ごとに並べる復号（`decode_rows`）が、1 本ずつの `decode` と同じ値を書き、同じ誤り（文まで）を返す。壊れ方が 2 つあれば、
    /// `decode` が先に出会うほう（上の行・行の切り出し・末尾の余り）。
    #[test]
    fn decoding_rows_in_parallel_matches_decoding_in_order() {
        let (w, h) = (37usize, 23usize);
        let good = packed_rows(w, h);
        let bad_row = |rows: &mut Vec<Vec<u8>>, y: usize, longer: bool| {
            if longer {
                rows[y].extend([0, 7]);
            } else {
                rows[y] = pack(&vec![1; w - 2]);
            }
        };
        let mut cases: Vec<(&str, Vec<u8>)> = vec![("good", rle(&good))];
        for (name, edits) in [
            ("long", vec![(5, true)]),
            ("short", vec![(9, false)]),
            ("two", vec![(12, true), (4, false)]),
            ("last", vec![(22, false)]),
        ] {
            let mut rows = good.clone();
            for (y, longer) in edits {
                bad_row(&mut rows, y, longer);
            }
            cases.push((name, rle(&rows)));
        }
        // 表は行の長さを大きく言うが、データが足りない（15 行目で切り出せない）。上の行が壊れていればそちらが先
        for bad in [None, Some(3), Some(18)] {
            let mut rows = good.clone();
            if let Some(y) = bad {
                bad_row(&mut rows, y, true);
            }
            let mut bytes = rle(&rows);
            let at = 2 + 15 * 2;
            let n = u16::from_be_bytes([bytes[at], bytes[at + 1]]) + 2000;
            bytes[at..at + 2].copy_from_slice(&n.to_be_bytes());
            cases.push(("cut", bytes));
        }
        // 末尾の余り（上の行が壊れていればそちらが先）
        for bad in [None, Some(20)] {
            let mut rows = good.clone();
            if let Some(y) = bad {
                bad_row(&mut rows, y, false);
            }
            let mut bytes = rle(&rows);
            bytes.extend([1, 2, 3]);
            cases.push(("tail", bytes));
        }
        // 直書きの途中で行が終わる（行の中の位置つきの誤り）
        let mut rows = good.clone();
        rows[7] = vec![10, 1, 2];
        cases.push(("literal", rle(&rows)));
        // 表が切れている・無圧縮（長さが合う・合わない）・未対応の圧縮・空
        cases.push(("table", rle(&good)[..20].to_vec()));
        let raw: Vec<u8> = (0..w * h).map(|i| (i * 7) as u8).collect();
        cases.push(("raw", [vec![0, 0], raw.clone()].concat()));
        cases.push(("raw short", [vec![0, 0], raw[1..].to_vec()].concat()));
        cases.push(("zip", vec![0, 2, 1, 2, 3]));
        cases.push(("empty", vec![0]));
        let limits = Limits::default();
        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            for (name, bytes) in &cases {
                for (component, stride) in [(0, 1), (2, 4)] {
                    let mut expected = vec![0xa5u8; w * h * stride];
                    let mut got = expected.clone();
                    let mut s = state(&limits);
                    let e = decode(
                        Reader::lenient(bytes),
                        w as u32,
                        h as u32,
                        &mut expected,
                        component,
                        stride,
                        &mut s,
                    );
                    let mut s2 = state(&limits);
                    let g = pool.install(|| {
                        decode_rows(
                            Reader::lenient(bytes),
                            w as u32,
                            h as u32,
                            &mut got,
                            component,
                            stride,
                            &mut s2,
                        )
                    });
                    assert_eq!(
                        format!("{g:?}"),
                        format!("{e:?}"),
                        "{name} {stride} {threads}"
                    );
                    assert_eq!(s2.notes, s.notes, "{name}");
                    if e.is_ok() {
                        assert!(got == expected, "{name} {stride} {threads}");
                    }
                    // 画素を書かずに確かめる形も、同じ誤り・知らせ（行を並べる道と 1 本の道の両方）。場所は ZIP など 1 本ずつ
                    // 復号するときだけ取る
                    for parallel_from in [0, usize::MAX] {
                        let (mut plane, mut s3) = (Vec::new(), state(&limits));
                        let c = pool.install(|| {
                            check_rows_from(
                                Reader::lenient(bytes),
                                w as u32,
                                h as u32,
                                &mut plane,
                                &mut s3,
                                parallel_from,
                            )
                        });
                        let what = format!("{name} {threads} {parallel_from}");
                        assert_eq!(format!("{c:?}"), format!("{e:?}"), "{what}");
                        assert_eq!(s3.notes, s.notes, "{what}");
                        let decoded_in_order = bytes.len() >= 2 && bytes[1] > 1;
                        assert_eq!(plane.is_empty(), !decoded_in_order, "{what}");
                    }
                }
            }
        }
        assert!(
            cases.iter().filter(|(n, _)| *n == "good").count() == 1
                && decode_rows(
                    Reader::lenient(&cases[0].1),
                    w as u32,
                    h as u32,
                    &mut vec![0; w * h],
                    0,
                    1,
                    &mut state(&limits)
                )
                .is_ok(),
            "壊れていない並びは読める"
        );
    }
}
