//! PSD の「写しとしての取り込み」（`psd::import_copy`）。実物の PSD は使わず、この試験が組み立てる PSD で確かめる。
//! 原本を保つ読み（`psd::read`）が PreserveOnly・Rejected にする PSD を、写しとしては取り込めること、
//! 取り込めないもの（PSB・RGB8 以外・予算を超える・壊れている）は理由つきで断ること、
//! 無視・落とした・変わるものが層の名前と機能の名前で知らされること。
use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use yolu_core::{Channel, Rect};
use yolu_io::psd::{
    self, Adjustment, BlendMode, CompatibilityMode, CopyOptions, CopyOutcome, CopyRefusal,
    Document, ImportAction, ImportDetail, ImportFeature, ImportNote, Layer, LayerKind, Limits,
    Mask, Unchecked,
};
use yolu_io::Error;

const MIB: u64 = 1024 * 1024;

// ───────── PSD の組み立て ─────────

#[derive(Clone)]
struct MaskSpec {
    /// 上・左・下・右。
    rect: (i32, i32, i32, i32),
    default: u8,
    flags: u8,
    pixels: Vec<u8>,
    /// マスクのチャンネル（-2）を持つか。false なら定義だけ。
    channel: bool,
}

#[derive(Clone)]
struct L {
    name: String,
    /// 上・左・下・右。
    rect: (i32, i32, i32, i32),
    /// 上の行から、RGBA。
    rgba: Vec<u8>,
    opacity: u8,
    clipping: u8,
    flags: u8,
    blend: [u8; 4],
    tags: Vec<([u8; 4], Vec<u8>)>,
    /// 空なら Blend-If の範囲なし。
    ranges: Vec<u8>,
    mask: Option<MaskSpec>,
    /// 0 RAW・1 RLE・2 ZIP・3 予測つき ZIP。
    compression: u16,
}
impl L {
    fn new(name: &str, rect: (i32, i32, i32, i32), color: [u8; 4]) -> L {
        let n = ((rect.2 - rect.0).max(0) * (rect.3 - rect.1).max(0)) as usize;
        L {
            name: name.into(),
            rect,
            rgba: color.iter().copied().cycle().take(n * 4).collect(),
            opacity: 255,
            clipping: 0,
            flags: 0,
            blend: *b"norm",
            tags: Vec::new(),
            ranges: Vec::new(),
            mask: None,
            compression: 0,
        }
    }
    fn tag(mut self, key: &[u8; 4], body: &[u8]) -> L {
        self.tags.push((*key, body.to_vec()));
        self
    }
    fn id(self, id: i32) -> L {
        self.tag(b"lyid", &id.to_be_bytes())
    }
    fn width(&self) -> usize {
        (self.rect.3 - self.rect.1).max(0) as usize
    }
    fn height(&self) -> usize {
        (self.rect.2 - self.rect.0).max(0) as usize
    }
}
/// 区切りと見出し（グループ）の 2 つの記録。下から上の並びに、`[区切り, 中身..., 見出し]` と入れる。
fn divider(id: i32) -> L {
    let mut l = L::new("</Layer group>", (0, 0, 0, 0), [0; 4]);
    l.tags.push((*b"lsct", 3u32.to_be_bytes().to_vec()));
    l.tags.push((*b"lyid", id.to_be_bytes().to_vec()));
    l
}
fn group(name: &str, id: i32) -> L {
    let mut l = L::new(name, (0, 0, 0, 0), [0; 4]);
    let mut body = 1u32.to_be_bytes().to_vec();
    body.extend(b"8BIMpass");
    l.blend = *b"norm";
    l.tags.push((*b"lsct", body));
    l.tags.push((*b"lyid", id.to_be_bytes().to_vec()));
    l
}

fn be16(v: u16) -> [u8; 2] {
    v.to_be_bytes()
}
fn be32(v: u32) -> [u8; 4] {
    v.to_be_bytes()
}
fn tagged(out: &mut Vec<u8>, key: &[u8; 4], body: &[u8]) {
    out.extend(b"8BIM");
    out.extend(key);
    out.extend(be32(body.len() as u32));
    out.extend(body);
    if body.len() % 2 == 1 {
        out.push(0)
    }
}
fn packbits(plane: &[u8], w: usize, h: usize) -> (Vec<u16>, Vec<u8>) {
    let (mut counts, mut data) = (Vec::new(), Vec::new());
    for y in 0..h {
        let row = &plane[y * w..(y + 1) * w];
        let before = data.len();
        for chunk in row.chunks(128) {
            data.push((chunk.len() - 1) as u8);
            data.extend(chunk);
        }
        counts.push((data.len() - before) as u16);
    }
    (counts, data)
}
fn zlib(bytes: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}
/// 行ごとに左隣との差にする（予測つき ZIP の前処理）。
fn predict(plane: &[u8], w: usize) -> Vec<u8> {
    let mut p = plane.to_vec();
    for row in p.chunks_mut(w.max(1)) {
        for x in (1..row.len()).rev() {
            row[x] = row[x].wrapping_sub(row[x - 1])
        }
    }
    p
}
/// 1 チャンネル（圧縮の種類 + データ）。
fn channel(plane: &[u8], w: usize, h: usize, compression: u16) -> Vec<u8> {
    let mut out = be16(compression).to_vec();
    match compression {
        0 => out.extend(plane),
        1 => {
            let (counts, data) = packbits(plane, w, h);
            for c in counts {
                out.extend(be16(c))
            }
            out.extend(data)
        }
        2 => out.extend(zlib(plane)),
        _ => out.extend(zlib(&predict(plane, w))),
    }
    out
}

struct Psd {
    width: u32,
    height: u32,
    layers: Vec<L>,
    resources: Vec<(u16, Vec<u8>)>,
    global_mask: Vec<u8>,
    global_tags: Vec<([u8; 4], Vec<u8>)>,
    /// 統合画像の圧縮（0 RAW・1 RLE・2 ZIP・3 予測つき ZIP）。
    merged_compression: u16,
    /// RGB のあとに足すアルファチャンネル（選択範囲など）の数。統合画像のチャンネルが増える。
    extra_channels: usize,
    /// 統合透明度を持つ（層の数を負にして、統合画像の 4 つ目のチャンネルを透明度にする。RGB は白へ寄せる）。
    merged_alpha: bool,
    /// 統合画像の透明度を、わざとこの一様な値にする（`merged_alpha` のとき）。
    merged_alpha_override: Option<u8>,
}
impl Psd {
    fn new(width: u32, height: u32, layers: Vec<L>) -> Psd {
        Psd {
            width,
            height,
            layers,
            resources: Vec::new(),
            global_mask: Vec::new(),
            global_tags: Vec::new(),
            merged_compression: 0,
            extra_channels: 0,
            merged_alpha: false,
            merged_alpha_override: None,
        }
    }
    /// 透明の上へ「通常」で重ねた統合画像（上の行から）。RGB は白へ寄せたものと、透明度の面。
    fn merged_with_alpha(&self) -> (Vec<u8>, Vec<u8>) {
        let (w, h) = (self.width as i32, self.height as i32);
        let n = (w * h) as usize;
        let mut color = vec![[0.0f64; 3]; n];
        let mut alpha = vec![0.0f64; n];
        for l in &self.layers {
            if l.flags & 2 != 0 || l.rect.0 == l.rect.2 {
                continue;
            }
            for y in 0..l.height() as i32 {
                for x in 0..l.width() as i32 {
                    let (px, py) = (l.rect.1 + x, l.rect.0 + y);
                    if px < 0 || py < 0 || px >= w || py >= h {
                        continue;
                    }
                    let s = (y as usize * l.width() + x as usize) * 4;
                    let sa = f64::from(l.rgba[s + 3]) / 255.0 * f64::from(l.opacity) / 255.0;
                    let d = (py * w + px) as usize;
                    let out = sa + alpha[d] * (1.0 - sa);
                    if out > 0.0 {
                        for (c, v) in color[d].iter_mut().enumerate() {
                            *v = (f64::from(l.rgba[s + c]) * sa + *v * alpha[d] * (1.0 - sa)) / out;
                        }
                    }
                    alpha[d] = out;
                }
            }
        }
        let mut rgb = Vec::with_capacity(n * 3);
        for (c, a) in color.iter().zip(&alpha) {
            for v in c {
                rgb.push((v * a + 255.0 * (1.0 - a) + 0.5).floor().clamp(0.0, 255.0) as u8)
            }
        }
        let a = alpha.iter().map(|a| (a * 255.0 + 0.5) as u8).collect();
        (rgb, a)
    }
    /// 通常の合成・不透明度だけで、白の上に重ねた統合画像（上の行から、RGB）。
    fn merged(&self) -> Vec<u8> {
        let (w, h) = (self.width as i32, self.height as i32);
        let mut out = vec![255u8; (w * h * 3) as usize];
        for l in &self.layers {
            if l.flags & 2 != 0 || l.rect.0 == l.rect.2 {
                continue;
            }
            for y in 0..l.height() as i32 {
                for x in 0..l.width() as i32 {
                    let (px, py) = (l.rect.1 + x, l.rect.0 + y);
                    if px < 0 || py < 0 || px >= w || py >= h {
                        continue;
                    }
                    let s = (y as usize * l.width() + x as usize) * 4;
                    let a = f64::from(l.rgba[s + 3]) / 255.0 * f64::from(l.opacity) / 255.0;
                    let d = ((py * w + px) * 3) as usize;
                    for c in 0..3 {
                        let below = f64::from(out[d + c]);
                        out[d + c] = (below + (f64::from(l.rgba[s + c]) - below) * a + 0.5) as u8;
                    }
                }
            }
        }
        out
    }
    fn build(&self) -> Vec<u8> {
        let mut info = Vec::new();
        // 層の数。統合透明度があるときは負（2 の補数）
        let count = self.layers.len() as i16;
        info.extend(be16((if self.merged_alpha { -count } else { count }) as u16));
        for l in &self.layers {
            info.extend(be32(l.rect.0 as u32));
            info.extend(be32(l.rect.1 as u32));
            info.extend(be32(l.rect.2 as u32));
            info.extend(be32(l.rect.3 as u32));
            let channels: Vec<i16> = if l.mask.as_ref().is_some_and(|m| m.channel) {
                vec![0, 1, 2, -1, -2]
            } else {
                vec![0, 1, 2, -1]
            };
            info.extend(be16(channels.len() as u16));
            for id in &channels {
                let (w, h) = if *id == -2 {
                    let m = l.mask.as_ref().unwrap();
                    (
                        (m.rect.3 - m.rect.1).max(0) as usize,
                        (m.rect.2 - m.rect.0).max(0) as usize,
                    )
                } else {
                    (l.width(), l.height())
                };
                let compression = if *id == -2 { 0 } else { l.compression };
                let len = if l.rect.0 == l.rect.2 && *id != -2 {
                    2
                } else {
                    self.channel_bytes(l, *id, w, h, compression).len()
                };
                info.extend(be16(*id as u16));
                info.extend(be32(len as u32));
            }
            info.extend(b"8BIM");
            info.extend(l.blend);
            info.extend([l.opacity, l.clipping, l.flags, 0]);
            let mut extra = Vec::new();
            match &l.mask {
                Some(m) => {
                    extra.extend(be32(20));
                    for v in [m.rect.0, m.rect.1, m.rect.2, m.rect.3] {
                        extra.extend(be32(v as u32))
                    }
                    extra.extend([m.default, m.flags, 0, 0]);
                }
                None => extra.extend(be32(0)),
            }
            extra.extend(be32(l.ranges.len() as u32));
            extra.extend(&l.ranges);
            let name: Vec<u8> = l.name.bytes().map(|b| if b < 128 { b } else { b'?' }).collect();
            extra.push(name.len() as u8);
            extra.extend(&name);
            extra.extend(vec![0; (4 - (name.len() + 1) % 4) % 4]);
            let units: Vec<u16> = l.name.encode_utf16().collect();
            let mut luni = be32(units.len() as u32).to_vec();
            for u in units {
                luni.extend(be16(u))
            }
            tagged(&mut extra, b"luni", &luni);
            for (key, body) in &l.tags {
                tagged(&mut extra, key, body)
            }
            info.extend(be32(extra.len() as u32));
            info.extend(extra);
        }
        for l in &self.layers {
            let ids: &[i16] = if l.mask.as_ref().is_some_and(|m| m.channel) {
                &[0, 1, 2, -1, -2]
            } else {
                &[0, 1, 2, -1]
            };
            for id in ids {
                let (w, h) = if *id == -2 {
                    let m = l.mask.as_ref().unwrap();
                    (
                        (m.rect.3 - m.rect.1).max(0) as usize,
                        (m.rect.2 - m.rect.0).max(0) as usize,
                    )
                } else {
                    (l.width(), l.height())
                };
                let compression = if *id == -2 { 0 } else { l.compression };
                if l.rect.0 == l.rect.2 && *id != -2 {
                    info.extend(be16(0));
                } else {
                    info.extend(self.channel_bytes(l, *id, w, h, compression))
                }
            }
        }
        if info.len() % 2 == 1 {
            info.push(0)
        }
        let mut lm = Vec::new();
        lm.extend(be32(info.len() as u32));
        lm.extend(info);
        lm.extend(be32(self.global_mask.len() as u32));
        lm.extend(&self.global_mask);
        for (key, body) in &self.global_tags {
            tagged(&mut lm, key, body)
        }
        let mut out = Vec::new();
        out.extend(b"8BPS");
        out.extend(be16(1));
        out.extend([0; 6]);
        out.extend(be16(3 + u16::from(self.merged_alpha) + self.extra_channels as u16));
        out.extend(be32(self.height));
        out.extend(be32(self.width));
        out.extend(be16(8));
        out.extend(be16(3));
        out.extend(be32(0));
        let mut res = Vec::new();
        for (id, body) in &self.resources {
            res.extend(b"8BIM");
            res.extend(be16(*id));
            res.extend([0, 0]);
            res.extend(be32(body.len() as u32));
            res.extend(body);
            if body.len() % 2 == 1 {
                res.push(0)
            }
        }
        out.extend(be32(res.len() as u32));
        out.extend(res);
        out.extend(be32(lm.len() as u32));
        out.extend(lm);
        let (w, h) = (self.width as usize, self.height as usize);
        let (merged, alpha) = if self.merged_alpha {
            let (rgb, a) = self.merged_with_alpha();
            (rgb, Some(a))
        } else {
            (self.merged(), None)
        };
        let mut planes: Vec<Vec<u8>> = (0..3)
            .map(|c| merged.iter().skip(c).step_by(3).copied().collect())
            .collect();
        if let Some(a) = alpha {
            planes.push(match self.merged_alpha_override {
                Some(v) => vec![v; w * h],
                None => a,
            })
        }
        planes.extend((0..self.extra_channels).map(|i| vec![i as u8 + 1; w * h]));
        match self.merged_compression {
            0 => {
                out.extend(be16(0));
                for p in &planes {
                    out.extend(p)
                }
            }
            1 => {
                out.extend(be16(1));
                let rows: Vec<_> = planes.iter().map(|p| packbits(p, w, h)).collect();
                for (counts, _) in &rows {
                    for c in counts {
                        out.extend(be16(*c))
                    }
                }
                for (_, data) in &rows {
                    out.extend(data)
                }
            }
            2 => {
                out.extend(be16(2));
                out.extend(zlib(&planes.concat()))
            }
            _ => {
                out.extend(be16(3));
                let predicted: Vec<u8> = planes.iter().flat_map(|p| predict(p, w)).collect();
                out.extend(zlib(&predicted))
            }
        }
        out
    }
    fn channel_bytes(&self, l: &L, id: i16, w: usize, h: usize, compression: u16) -> Vec<u8> {
        let plane: Vec<u8> = if id == -2 {
            l.mask.as_ref().unwrap().pixels.clone()
        } else {
            let c = if id == -1 { 3 } else { id as usize };
            l.rgba.iter().skip(c).step_by(4).copied().collect()
        };
        channel(&plane, w, h, compression)
    }
}

fn run(bytes: &[u8], budget: u64) -> CopyOutcome {
    run_with(bytes, budget, None)
}
fn run_with(bytes: &[u8], budget: u64, cancel: Option<&AtomicBool>) -> CopyOutcome {
    psd::import_copy(
        &mut Cursor::new(bytes.to_vec()),
        &CopyOptions {
            source_budget: budget,
            cancel,
        },
    )
    .unwrap()
}
fn imported(bytes: &[u8]) -> (yolu_core::Document, Vec<ImportNote>) {
    match run(bytes, 2048 * MIB) {
        CopyOutcome::Imported(i) => (i.document, i.notes),
        CopyOutcome::Refused(why) => panic!("断られた: {}", why.message()),
    }
}
fn refused(bytes: &[u8], budget: u64) -> CopyRefusal {
    match run(bytes, budget) {
        CopyOutcome::Refused(why) => why,
        CopyOutcome::Imported(_) => panic!("取り込めてしまった"),
    }
}
fn note(notes: &[ImportNote], feature: ImportFeature) -> Option<&ImportNote> {
    notes.iter().find(|n| n.feature == feature)
}
fn feature_set(notes: &[ImportNote]) -> Vec<(ImportFeature, ImportAction)> {
    notes.iter().map(|n| (n.feature, n.action)).collect()
}
fn layer_by_name<'a>(d: &'a yolu_core::Document, name: &str) -> &'a yolu_core::Layer {
    d.layers()
        .iter()
        .find(|l| l.name() == name)
        .unwrap_or_else(|| panic!("{name} が無い"))
}
fn composite(d: &yolu_core::Document) -> Vec<u8> {
    d.composite(Rect::new(0, 0, d.width(), d.height())).unwrap()
}
/// 層の左上の画素（PSD の上の行から）。
fn pixel(d: &yolu_core::Document, name: &str, x: u32, y: u32) -> [u8; 4] {
    let l = layer_by_name(d, name);
    let p = l
        .pixel(Channel::Color, x, d.height() - 1 - y)
        .unwrap();
    [p.r, p.g, p.b, p.a]
}
fn red(name: &str, rect: (i32, i32, i32, i32)) -> L {
    L::new(name, rect, [255, 0, 0, 255])
}
fn key(k: &[u8; 4]) -> ImportDetail {
    ImportDetail::Key(*k)
}

// ───────── 取り込める PSD（原本を保つ読みは断るもの） ─────────

#[test]
fn what_the_preserving_reader_refuses_imports_as_a_copy_with_each_thing_told() {
    // CLIP STUDIO・Photoshop の書き出しに見られる、合成に効かない情報だけの PSD
    let mut psd = Psd::new(
        8,
        8,
        vec![
            red("下", (0, 0, 8, 8)).tag(b"tsly", &[0, 0, 0, 0]),
            L::new("上", (2, 2, 6, 6), [0, 0, 255, 255]).tag(b"lclr", &[0, 1, 0, 0, 0, 0, 0, 0]),
        ],
    );
    // lyid を持たない 2 層。全体マスク・画像リソース（チャンネル名と解像度）
    psd.global_mask = vec![0; 14];
    psd.resources = vec![(1006, b"\x04abcd".to_vec()), (1005, vec![0; 16])];
    let bytes = psd.build();
    // 原本を保つ読みは、どれも編集できない内容として原本の保持だけにする
    let strict = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(strict.mode(), CompatibilityMode::PreserveOnly);
    let codes: Vec<&str> = strict.diagnostics().iter().map(|d| d.code.as_str()).collect();
    for code in ["GlobalMask", "ImageResource", "LayerIdentity", "TransparencyShapes"] {
        assert!(codes.contains(&code), "{code}: {codes:?}");
    }
    // 写しとしては取り込める。無視したものが、機能の名前と層の名前で並ぶ
    let (d, notes) = imported(&bytes);
    assert_eq!(d.layers().len(), 2);
    assert_eq!(
        feature_set(&notes),
        [
            (ImportFeature::ImageResources, ImportAction::Ignored),
            (ImportFeature::TransparencyShapes, ImportAction::Ignored),
            (ImportFeature::LayerColorLabel, ImportAction::Ignored),
            (ImportFeature::LayerIds, ImportAction::Ignored),
            (ImportFeature::GlobalMask, ImportAction::Ignored),
        ],
        "{notes:?}"
    );
    assert_eq!(note(&notes, ImportFeature::ImageResources).unwrap().count, 2);
    assert_eq!(
        note(&notes, ImportFeature::TransparencyShapes).unwrap().layers,
        ["下"]
    );
    assert_eq!(
        note(&notes, ImportFeature::LayerColorLabel).unwrap().layers,
        ["上"]
    );
    // 画素は PSD のとおり（赤の上に青）
    assert_eq!(pixel(&d, "下", 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&d, "上", 3, 3), [0, 0, 255, 255]);
}

#[test]
fn missing_and_duplicate_layer_ids_get_new_ids_and_are_never_repaired_by_name() {
    // 同じ名前の 2 層（名前で対応付けたら取り違える）、lyid が欠落・重複・0・負の層
    let psd = Psd::new(
        4,
        4,
        vec![
            red("同じ名前", (0, 0, 2, 2)).id(5),
            L::new("同じ名前", (2, 2, 4, 4), [0, 255, 0, 255]).id(5),
            L::new("欠落", (0, 2, 2, 4), [0, 0, 255, 255]),
            L::new("ゼロ", (2, 0, 4, 2), [9, 9, 9, 255]).id(0),
            L::new("負", (1, 1, 3, 3), [7, 7, 7, 255]).id(-4),
        ],
    );
    let bytes = psd.build();
    assert_eq!(
        psd::read(&bytes, &Limits::default()).unwrap().mode(),
        CompatibilityMode::PreserveOnly
    );
    let (d, notes) = imported(&bytes);
    assert_eq!(d.layers().len(), 5);
    // 層 ID（上位 32 bit）は正で、すべて違う
    let ids: Vec<u128> = d.layers().iter().map(|l| l.id().0 >> 96).collect();
    assert!(ids.iter().all(|i| *i > 0), "{ids:?}");
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 5, "{ids:?}");
    // 最初に出てきた lyid 5 はそのまま。重複・欠落・0・負だけが振り直された
    assert_eq!(ids[0], 5);
    let n = note(&notes, ImportFeature::LayerIds).unwrap();
    // 写しとして取り込むので、ID の振り直しは見え方にも内容にも効かない（無視。確かめの窓は出さない）
    assert_eq!(n.action, ImportAction::Ignored);
    assert_eq!(n.count, 4);
    assert_eq!(n.layers, ["同じ名前", "欠落", "ゼロ", "負"]);
    // 同じ名前の 2 層は、位置で区別されたまま（画素が入れ替わらない）
    assert_eq!(pixel(&d, "同じ名前", 0, 0), [255, 0, 0, 255]);
    let second = &d.layers()[1];
    assert_eq!(second.name(), "同じ名前");
    let p = second.pixel(Channel::Color, 3, 0).unwrap();
    assert_eq!([p.r, p.g, p.b, p.a], [0, 255, 0, 255]);
}

#[test]
fn only_missing_duplicate_and_invalid_ids_are_renumbered_even_when_valid_ones_follow() {
    // 下から [欠落, 1, 2, 2（重複）, 4]。欠落の層に 1 から振ると、あとにある有効な 1・2 とぶつかる
    let psd = Psd::new(
        4,
        4,
        vec![
            L::new("欠落", (0, 0, 2, 2), [9, 9, 9, 255]),
            red("一", (0, 2, 2, 4)).id(1),
            red("二", (2, 0, 4, 2)).id(2),
            red("二の重複", (2, 2, 4, 4)).id(2),
            red("四", (1, 1, 3, 3)).id(4),
        ],
    );
    let (d, notes) = imported(&psd.build());
    let ids: Vec<u128> = d.layers().iter().map(|l| l.id().0 >> 96).collect();
    // 有効で一意な 1・2・4 は変わらない。振り直されたのは欠落と重複の 2 層だけ（使われていない 3・5 を振る）
    assert_eq!([ids[1], ids[2], ids[4]], [1, 2, 4], "{ids:?}");
    assert_eq!([ids[0], ids[3]], [3, 5], "{ids:?}");
    let n = note(&notes, ImportFeature::LayerIds).unwrap();
    assert_eq!((n.count, n.layers.clone()), (2, vec!["欠落".to_string(), "二の重複".to_string()]));
    // 区切りの ID とも重ならない（区切りが 1 を使うので、欠落の層は 1 を避ける）
    let psd = Psd::new(
        4,
        4,
        vec![
            L::new("欠落", (0, 0, 2, 2), [9, 9, 9, 255]),
            divider(1),
            red("中", (0, 2, 2, 4)).id(2),
            group("組", 3),
        ],
    );
    let (d, notes) = imported(&psd.build());
    let ids: Vec<u128> = d.layers().iter().map(|l| l.id().0 >> 96).collect();
    assert_eq!(ids, [4, 2, 3], "{ids:?}");
    assert_eq!(note(&notes, ImportFeature::LayerIds).unwrap().layers, ["欠落"]);
}

#[test]
fn effects_that_cannot_be_evaluated_keep_the_layer_pixels_and_say_what_changes() {
    let psd = Psd::new(
        8,
        8,
        vec![
            red("効果", (0, 0, 4, 4)).tag(b"lfx2", &[0; 16]),
            red("スマート", (0, 4, 4, 8)).tag(b"SoLd", &[0; 8]),
            red("文字", (4, 0, 8, 4)).tag(b"TySh", &[0; 8]),
            red("ベクター", (4, 4, 8, 8)).tag(b"vmsk", &[0; 8]),
            red("グラデ塗り", (2, 2, 6, 6)).tag(b"GdFl", &[0; 8]),
            red("メタ", (0, 0, 2, 2)).tag(b"lnsr", &[0; 4]).tag(b"xyzw", &[0; 4]),
        ],
    );
    let bytes = psd.build();
    assert_eq!(
        psd::read(&bytes, &Limits::default()).unwrap().mode(),
        CompatibilityMode::PreserveOnly
    );
    let (d, notes) = imported(&bytes);
    assert_eq!(d.layers().len(), 6, "どの層も残る");
    for (feature, action, layer, at) in [
        (ImportFeature::LayerEffects, ImportAction::Changed, "効果", (0, 0)),
        (ImportFeature::SmartObject, ImportAction::Dropped, "スマート", (4, 0)),
        (ImportFeature::TextLayer, ImportAction::Dropped, "文字", (0, 4)),
        (ImportFeature::VectorMask, ImportAction::Changed, "ベクター", (4, 4)),
        (ImportFeature::FillSettings, ImportAction::Dropped, "グラデ塗り", (3, 3)),
    ] {
        let n = note(&notes, feature).unwrap_or_else(|| panic!("{feature:?}: {notes:?}"));
        assert_eq!((n.action, n.layers.as_slice()), (action, [layer.to_string()].as_slice()));
        // 層の画素は PSD のまま
        assert_eq!(pixel(&d, layer, at.0, at.1), [255, 0, 0, 255], "{layer}");
    }
    // 未知のタグは、持たないだけとしてキーつきで知らせる
    let meta = note(&notes, ImportFeature::LayerMetadata).unwrap();
    assert_eq!(meta.action, ImportAction::Ignored);
    assert_eq!(meta.details, [key(b"lnsr"), key(b"xyzw")]);
}

#[test]
fn an_unsupported_adjustment_drops_the_layer_but_a_supported_one_stays() {
    let mut inv = L::new("反転", (0, 0, 0, 0), [0; 4]);
    inv.tags.push((*b"nvrt", Vec::new()));
    let mut selective = L::new("色の選択", (0, 0, 0, 0), [0; 4]);
    selective.tags.push((*b"selc", vec![0; 8]));
    let psd = Psd::new(4, 4, vec![red("下", (0, 0, 4, 4)), selective, inv]);
    let (d, notes) = imported(&psd.build());
    let names: Vec<&str> = d.layers().iter().map(|l| l.name()).collect();
    assert_eq!(names, ["下", "反転"], "未対応の調整は層ごと落とす");
    let n = note(&notes, ImportFeature::UnsupportedAdjustment).unwrap();
    assert_eq!((n.action, n.layers.clone(), n.details.clone()), (
        ImportAction::Dropped,
        vec!["色の選択".to_string()],
        vec![key(b"selc")]
    ));
}

#[test]
fn a_legacy_brightness_contrast_alone_is_an_adjustment_that_cannot_be_kept_and_is_dropped() {
    // 旧式の brit だけ（新しい式の CgEd が無い）: 原本を保つ読みは保つだけ。写しとしては層ごと落として、そのキーを知らせる
    let mut legacy = L::new("旧式", (0, 0, 0, 0), [0; 4]);
    legacy.tags.push((*b"brit", vec![0, 10, 0, 10, 0, 0, 0, 0]));
    let psd = Psd::new(4, 4, vec![red("下", (0, 0, 4, 4)), legacy]);
    let bytes = psd.build();
    assert_eq!(
        psd::read(&bytes, &Limits::default()).unwrap().mode(),
        CompatibilityMode::PreserveOnly
    );
    let (d, notes) = imported(&bytes);
    assert_eq!(d.layers().len(), 1);
    let n = note(&notes, ImportFeature::UnsupportedAdjustment).unwrap();
    assert_eq!((n.action, n.layers.clone(), n.details.clone()), (
        ImportAction::Dropped,
        vec!["旧式".to_string()],
        vec![key(b"brit")]
    ));
}

#[test]
fn image_resources_that_are_not_resource_blocks_are_ignored_not_refused() {
    let mut psd = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
    psd.resources = vec![(1005, vec![0; 16])];
    let mut bytes = psd.build();
    // 画像リソースの区間の先頭（シグネチャ 8BIM）を壊す。区間の長さは変えない
    let at = bytes.windows(4).position(|w| w == b"8BIM").unwrap();
    bytes[at..at + 4].copy_from_slice(b"XXXX");
    let (d, notes) = imported(&bytes);
    assert_eq!(d.layers().len(), 1);
    assert_eq!(note(&notes, ImportFeature::ImageResources).unwrap().action, ImportAction::Ignored);
}

#[test]
fn a_color_profile_that_is_not_srgb_and_a_stretched_pixel_aspect_change_the_values() {
    let mut psd = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
    // 1039 は ICC（ここでは読めない中身 = sRGB ではない）、1064 は画素の縦横比（版 + 倍精度の 2.0）
    let mut aspect = be32(2).to_vec();
    aspect.extend(2.0f64.to_be_bytes());
    psd.resources = vec![(1039, vec![1, 2, 3, 4]), (1064, aspect)];
    let (_, notes) = imported(&psd.build());
    assert_eq!(note(&notes, ImportFeature::ColorProfile).unwrap().action, ImportAction::Changed);
    assert_eq!(note(&notes, ImportFeature::PixelAspect).unwrap().action, ImportAction::Changed);
    assert!(note(&notes, ImportFeature::ImageResources).is_none(), "{notes:?}");
    // 正方形（1.0）なら知らせない
    let mut square = be32(2).to_vec();
    square.extend(1.0f64.to_be_bytes());
    let mut psd = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
    psd.resources = vec![(1064, square)];
    let (_, notes) = imported(&psd.build());
    assert!(note(&notes, ImportFeature::PixelAspect).is_none(), "{notes:?}");
}

#[test]
fn unsupported_blend_modes_become_normal_and_fill_opacity_multiplies_into_opacity() {
    let mut dissolve = red("ディゾルブ", (0, 0, 4, 4));
    dissolve.blend = *b"diss";
    let mut half = red("塗り半分", (0, 0, 4, 4)).tag(b"iOpa", &[128, 0, 0, 0]);
    half.opacity = 200;
    let psd = Psd::new(4, 4, vec![dissolve, half]);
    let (d, notes) = imported(&psd.build());
    assert_eq!(layer_by_name(&d, "ディゾルブ").blend_mode(), yolu_core::BlendMode::Normal);
    let n = note(&notes, ImportFeature::BlendMode).unwrap();
    assert_eq!((n.action, n.details.clone()), (ImportAction::Changed, vec![key(b"diss")]));
    // 不透明度 200/255 × 塗り 128/255 → 100/255
    let o = layer_by_name(&d, "塗り半分").opacity();
    assert!((o - 100.0 / 255.0).abs() < 1e-9, "{o}");
    assert_eq!(note(&notes, ImportFeature::FillOpacity).unwrap().action, ImportAction::Changed);
}

#[test]
fn a_blend_if_range_and_mask_extras_change_the_look_and_are_listed() {
    let mut l = red("条件", (0, 0, 4, 4));
    // 既定でないブレンド範囲（下の層の側を 10〜200 だけ）
    l.ranges = vec![0, 0, 255, 255, 10, 20, 200, 210, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255];
    let mut masked = red("マスク", (0, 0, 4, 4));
    masked.mask = Some(MaskSpec {
        rect: (0, 0, 4, 4),
        default: 255,
        // 位置・反転のフラグ
        flags: 1 | 4,
        pixels: vec![255; 16],
        channel: true,
    });
    let psd = Psd::new(4, 4, vec![l, masked]);
    let (d, notes) = imported(&psd.build());
    assert_eq!(d.layers().len(), 2);
    assert_eq!(note(&notes, ImportFeature::BlendIf).unwrap().layers, ["条件"]);
    let n = note(&notes, ImportFeature::MaskFlags).unwrap();
    assert_eq!((n.action, n.layers.clone()), (ImportAction::Changed, vec!["マスク".to_string()]));
    assert!(layer_by_name(&d, "マスク").mask().is_some());
}

// ───────── 画布の外・マスク・構造 ─────────

#[test]
fn blend_settings_collapsed_groups_and_lock_bits_are_each_told() {
    let mut closed = group("閉じた組", 7);
    let mut body = 2u32.to_be_bytes().to_vec();
    body.extend(b"8BIMpass");
    closed.tags.retain(|t| &t.0 != b"lsct");
    closed.tags.push((*b"lsct", body));
    let psd = Psd::new(
        4,
        4,
        vec![
            divider(70),
            red("中", (0, 0, 4, 4)).id(1)
                .tag(b"clbl", &[0, 0, 0, 0])
                .tag(b"infx", &[1, 0, 0, 0])
                .tag(b"knko", &[1, 0, 0, 0])
                .tag(b"brst", &[0, 0, 0, 0, 0, 0, 0, 1])
                .tag(b"lspf", &be32(0x8)),
            closed,
        ],
    );
    let (d, notes) = imported(&psd.build());
    assert_eq!(d.layers().len(), 2);
    for (feature, action) in [
        (ImportFeature::ClippedBlend, ImportAction::Changed),
        (ImportFeature::InteriorBlend, ImportAction::Changed),
        (ImportFeature::Knockout, ImportAction::Changed),
        (ImportFeature::ChannelRestrictions, ImportAction::Changed),
        (ImportFeature::LayerLockBits, ImportAction::Ignored),
        (ImportFeature::CollapsedGroup, ImportAction::Ignored),
    ] {
        let n = note(&notes, feature).unwrap_or_else(|| panic!("{feature:?}: {notes:?}"));
        assert_eq!(n.action, action, "{feature:?}");
    }
    assert_eq!(note(&notes, ImportFeature::CollapsedGroup).unwrap().layers, ["閉じた組"]);
    assert_eq!(note(&notes, ImportFeature::Knockout).unwrap().layers, ["中"]);
}

#[test]
fn inconsistent_masks_and_sloppy_padding_do_not_refuse_the_file() {
    // マスクの定義だけでチャンネルが無い層（値は分からないので既定値だけ）・既定値が 0・255 以外のマスク
    let mut no_channel = red("定義だけ", (0, 0, 4, 4));
    no_channel.mask = Some(MaskSpec { rect: (0, 0, 4, 4), default: 255, flags: 0, pixels: Vec::new(), channel: false });
    let mut gray = red("中間", (0, 0, 4, 4));
    gray.mask = Some(MaskSpec { rect: (1, 1, 3, 3), default: 100, flags: 0, pixels: vec![255; 4], channel: true });
    // 失うものが無いマスク: 矩形が空の定義だけ・既定値が 0 と 255 のマスク
    let mut empty = red("空の矩形", (0, 0, 4, 4));
    empty.mask = Some(MaskSpec { rect: (0, 0, 0, 0), default: 255, flags: 0, pixels: Vec::new(), channel: false });
    let mut hidden = red("既定は 0", (0, 0, 4, 4));
    hidden.mask = Some(MaskSpec { rect: (1, 1, 3, 3), default: 0, flags: 0, pixels: vec![255; 4], channel: true });
    let bytes = Psd::new(4, 4, vec![no_channel, gray, empty, hidden]).build();
    assert_eq!(psd::read(&bytes, &Limits::default()).unwrap().mode(), CompatibilityMode::Rejected);
    let (d, notes) = imported(&bytes);
    assert_eq!(d.layers().len(), 4);
    // 見え方が変わる 2 つは、機能と層の名前で知らされる（黙って寄せない）
    let n = note(&notes, ImportFeature::MaskWithoutPixels).unwrap_or_else(|| panic!("{notes:?}"));
    assert_eq!((n.action, n.layers.clone(), n.count), (ImportAction::Changed, vec!["定義だけ".to_string()], 1));
    let n = note(&notes, ImportFeature::MaskDefault).unwrap_or_else(|| panic!("{notes:?}"));
    assert_eq!((n.action, n.layers.clone(), n.count), (ImportAction::Changed, vec!["中間".to_string()], 1));
    // 値の無いマスクは何も隠さない（既定値 255）
    let m = layer_by_name(&d, "定義だけ").mask().unwrap();
    assert_eq!(m.factor_at(0, 0).unwrap(), 1.0);
    // 既定値 100 は 0 に寄せる（矩形の外は隠す。矩形の中は 255 のまま見える）
    let m = layer_by_name(&d, "中間").mask().unwrap();
    assert_eq!(m.factor_at(0, 0).unwrap(), 0.0);
    assert_eq!(m.factor_at(1, 1).unwrap(), 1.0);
    // 既定値 0 と 255 は、寄せていないので知らせない
    assert_eq!(layer_by_name(&d, "既定は 0").mask().unwrap().factor_at(0, 0).unwrap(), 0.0);
    // 既定値が 128 以上なら 255 へ寄せる
    let mut light = red("明るめ", (0, 0, 4, 4));
    light.mask = Some(MaskSpec { rect: (1, 1, 3, 3), default: 200, flags: 0, pixels: vec![0; 4], channel: true });
    let (d, notes) = imported(&Psd::new(4, 4, vec![light]).build());
    assert_eq!(layer_by_name(&d, "明るめ").mask().unwrap().factor_at(0, 0).unwrap(), 1.0);
    assert_eq!(note(&notes, ImportFeature::MaskDefault).unwrap().layers, ["明るめ"]);
    // 余白（ゼロであるべき所）がゼロでない PSD。層名の Pascal 文字列の余白（"a" の後ろ）を 0 でない値にする
    let mut sloppy = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]).build();
    let at = sloppy.windows(2).position(|w| w == [1, b'a']).unwrap() + 2;
    sloppy[at] = 0x7f;
    let (d, _) = imported(&sloppy);
    assert_eq!(d.layers()[0].name(), "a");
    assert!(matches!(
        psd::read(&sloppy, &Limits::default()).map(|r| r.mode()),
        Ok(CompatibilityMode::Rejected) | Err(_)
    ));
}

#[test]
fn pixels_outside_the_canvas_are_cut_and_only_a_real_loss_is_told() {
    let mut wide = red("はみ出す", (-2, -2, 6, 6));
    // 画布の外の画素は透明にして、左上の 4×4（画布の内側）だけ赤にする
    for y in 0..8usize {
        for x in 0..8usize {
            let inside = x >= 2 && y >= 2 && x < 6 && y < 6;
            let i = (y * 8 + x) * 4;
            wide.rgba[i..i + 4].copy_from_slice(&if inside { [255, 0, 0, 255] } else { [0, 0, 0, 0] });
        }
    }
    let (d, notes) = imported(&Psd::new(4, 4, vec![wide.clone()]).build());
    assert!(note(&notes, ImportFeature::OutsideCanvas).is_none(), "{notes:?}");
    assert_eq!(pixel(&d, "はみ出す", 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&d, "はみ出す", 3, 3), [255, 0, 0, 255]);
    // 外にも見える画素があれば、落としたものとして層の名前つきで知らせる
    let mut visible = wide;
    visible.rgba[3] = 255;
    let (_, notes) = imported(&Psd::new(4, 4, vec![visible]).build());
    let n = note(&notes, ImportFeature::OutsideCanvas).unwrap();
    assert_eq!((n.action, n.layers.clone()), (ImportAction::Dropped, vec!["はみ出す".to_string()]));
}

#[test]
fn groups_masks_clipping_locks_and_empty_layers_come_across() {
    let mut masked = red("マスク", (0, 0, 4, 4));
    masked.mask = Some(MaskSpec {
        rect: (0, 0, 4, 4),
        default: 255,
        flags: 0,
        pixels: (0..16).map(|i| if i % 2 == 0 { 255 } else { 0 }).collect(),
        channel: true,
    });
    let mut clip = L::new("クリップ", (0, 0, 4, 4), [0, 255, 0, 255]);
    clip.clipping = 1;
    let psd = Psd::new(
        4,
        4,
        vec![
            divider(90),
            red("中", (0, 0, 4, 4)).id(1),
            L::new("空", (0, 0, 0, 0), [0; 4]).id(2),
            group("組", 3),
            masked.id(4),
            clip.id(5).tag(b"lspf", &be32(1)),
        ],
    );
    let (d, notes) = imported(&psd.build());
    // この組み立て方の統合画像はマスク・クリッピングを重ねないので、照合の差は数えない
    assert!(
        notes
            .iter()
            .all(|n| n.action == ImportAction::Ignored || matches!(n.feature, ImportFeature::CompositeDiffers { .. })),
        "{notes:?}"
    );
    let names: Vec<&str> = d.layers().iter().map(|l| l.name()).collect();
    assert_eq!(names, ["中", "空", "組", "マスク", "クリップ"]);
    let g = layer_by_name(&d, "組");
    assert!(g.is_group());
    assert_eq!(layer_by_name(&d, "中").parent(), Some(g.id()));
    assert_eq!(layer_by_name(&d, "空").parent(), Some(g.id()));
    assert_eq!(layer_by_name(&d, "マスク").parent(), None);
    assert!(layer_by_name(&d, "クリップ").clipping());
    assert!(layer_by_name(&d, "クリップ").locks().contains(yolu_core::LayerLocks::TRANSPARENCY));
    // 組の区切りの ID は層 ID に持ち、書き出し直すと同じ区切りになる
    let divider = (((g.id().0 >> 80) & 0xffff) | (((g.id().0 >> 64) & 0xffff) << 16)) as u32;
    assert_eq!(divider, 90);
    let mask = layer_by_name(&d, "マスク").mask().unwrap();
    assert!(mask.enabled());
}

// ───────── 圧縮 ─────────

#[test]
fn raw_rle_and_zip_channels_import_the_same_pixels() {
    let mut pixels = Vec::new();
    for y in 0..6u8 {
        for x in 0..9u8 {
            pixels.extend([x * 20, y * 40, x ^ y, 255 - x * 3]);
        }
    }
    let mut seen = Vec::new();
    for compression in 0..=3 {
        let mut l = L::new("絵", (0, 0, 6, 9), [0; 4]);
        l.rgba = pixels.clone();
        l.compression = compression;
        let mut psd = Psd::new(9, 6, vec![l]);
        psd.merged_compression = compression;
        let (d, _) = imported(&psd.build());
        let layer = layer_by_name(&d, "絵");
        let all: Vec<[u8; 4]> = (0..6)
            .flat_map(|y| (0..9).map(move |x| (x, y)))
            .map(|(x, y)| {
                let p = layer.pixel(Channel::Color, x, 5 - y).unwrap();
                [p.r, p.g, p.b, p.a]
            })
            .collect();
        seen.push(all);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]));
    assert_eq!(&seen[0][1], &[20, 0, 1, 252]);
}

// ───────── 統合画像との照合 ─────────

#[test]
fn a_clean_psd_matches_its_merged_image_and_a_wrong_one_reports_the_difference() {
    let mut psd = Psd::new(
        8,
        8,
        vec![red("下", (0, 0, 8, 8)), L::new("上", (2, 2, 6, 6), [0, 0, 255, 255])],
    );
    let (d, notes) = imported(&psd.build());
    assert!(
        !notes.iter().any(|n| matches!(n.feature, ImportFeature::CompositeDiffers { .. } | ImportFeature::CompositeUnchecked(_))),
        "{notes:?}"
    );
    assert_eq!(composite(&d).len(), 8 * 8 * 4);
    // 統合画像と合成が違う PSD（上の層を隠してから統合画像だけ元のまま）
    psd.layers[1].flags = 2;
    let mut bytes = psd.build();
    // 隠す前の統合画像へ差し替える
    let shown = {
        psd.layers[1].flags = 0;
        psd.build()
    };
    let tail = 2 + 3 * 64;
    let at = bytes.len() - tail;
    bytes[at..].copy_from_slice(&shown[shown.len() - tail..]);
    let (_, notes) = imported(&bytes);
    let n = notes
        .iter()
        .find_map(|n| match n.feature {
            ImportFeature::CompositeDiffers { max_diff, differing, total } => {
                Some((n.action, max_diff, differing, total))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("{notes:?}"));
    // 上の層の 4×4 が、青から赤へ
    assert_eq!((n.0, n.1, n.2, n.3), (ImportAction::Changed, 255, 16, 64));
}

#[test]
fn alpha_channels_in_the_merged_image_are_skipped_not_refused() {
    // RGB のあとに選択範囲のアルファチャンネルを 2 つ持つ PSD。原本を保つ読みは RGB8 の 3・4 チャンネル以外として保つだけ
    for compression in 0..=2 {
        let mut psd = Psd::new(8, 8, vec![red("下", (0, 0, 8, 8)), L::new("上", (2, 2, 6, 6), [0, 0, 255, 255])]);
        psd.extra_channels = 2;
        psd.merged_compression = compression;
        let bytes = psd.build();
        assert_eq!(
            psd::read(&bytes, &Limits::default()).unwrap().mode(),
            CompatibilityMode::PreserveOnly
        );
        let (d, notes) = imported(&bytes);
        assert_eq!(d.layers().len(), 2);
        let n = note(&notes, ImportFeature::ExtraChannel).unwrap_or_else(|| panic!("{notes:?}"));
        assert_eq!(n.action, ImportAction::Ignored);
        // 統合画像の色は照合に使えて、差は無い
        assert!(
            !notes.iter().any(|n| matches!(n.feature, ImportFeature::CompositeDiffers { .. } | ImportFeature::CompositeUnchecked(_))),
            "圧縮 {compression}: {notes:?}"
        );
    }
}

#[test]
fn a_missing_or_unreadable_merged_image_is_told_not_refused() {
    let psd = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
    let bytes = psd.build();
    // 統合画像が無い（レイヤーとマスクの情報で終わる）
    let cut = bytes.len() - (2 + 3 * 16);
    let (_, notes) = imported(&bytes[..cut]);
    assert!(note(&notes, ImportFeature::CompositeUnchecked(Unchecked::NoComposite)).is_some(), "{notes:?}");
    // 未対応の圧縮
    let mut weird = bytes.clone();
    let at = bytes.len() - (2 + 3 * 16);
    weird[at..at + 2].copy_from_slice(&be16(9));
    let (_, notes) = imported(&weird);
    assert!(note(&notes, ImportFeature::CompositeUnchecked(Unchecked::Unreadable)).is_some(), "{notes:?}");
    // 画布が予算に比べて大きいときは、照らさずに知らせる
    let wide = Psd::new(256, 256, vec![red("a", (0, 0, 1, 1))]).build();
    let notes = match run(&wide, 300 * 1024) {
        CopyOutcome::Imported(i) => i.notes,
        CopyOutcome::Refused(why) => panic!("{}", why.message()),
    };
    assert!(note(&notes, ImportFeature::CompositeUnchecked(Unchecked::Budget)).is_some(), "{notes:?}");
}

#[test]
fn a_merged_image_cut_short_is_unreadable_for_every_compression_and_the_layers_still_come() {
    // 層のデータは完全で、末尾の統合画像だけが切れている PSD。RAW・RLE・ZIP のどれでも、層は取り込まれ、照合を省略と知らされる
    for compression in [0u16, 1, 2, 3] {
        let mut psd = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
        psd.merged_compression = compression;
        let whole = psd.build();
        // 統合画像の始まり（圧縮の種類の前）は、同じ層で組んだ RAW の PSD の終わりから 2 + 3 面ぶんを引いた所
        let mut raw = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
        raw.merged_compression = 0;
        let start = raw.build().len() - (2 + 3 * 16);
        // 圧縮の種類のあと・先頭に近い所・データの半ばで切る（ZIP の末尾の検査値だけが欠けても、画素は全部読めるので切れていない）
        for cut in [start + 2, start + 2 + 5, start + 2 + (whole.len() - start - 2) / 2] {
            if cut >= whole.len() {
                continue;
            }
            let (d, notes) = imported(&whole[..cut]);
            assert_eq!(d.layers().len(), 1, "圧縮 {compression}・{cut} バイト: 層が取り込まれない");
            assert!(
                note(&notes, ImportFeature::CompositeUnchecked(Unchecked::Unreadable)).is_some()
                    || note(&notes, ImportFeature::CompositeUnchecked(Unchecked::NoComposite)).is_some(),
                "圧縮 {compression}・{cut} バイト: {notes:?}"
            );
            assert!(
                note(&notes, ImportFeature::CompositeDiffers { max_diff: 0, differing: 0, total: 0 }).is_none(),
                "{notes:?}"
            );
        }
    }
    // RLE は、表の途中・行の途中のどちらも、断らず「読めない」になる（以前は RLE だけが取り込み全体を断っていた）
    let mut psd = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
    psd.merged_compression = 1;
    let rle = psd.build();
    let mut raw = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]);
    raw.merged_compression = 0;
    let start = raw.build().len() - (2 + 3 * 16);
    for cut in [start + 2, start + 2 + 5, rle.len() - 3] {
        let (_, notes) = imported(&rle[..cut]);
        assert!(
            note(&notes, ImportFeature::CompositeUnchecked(Unchecked::Unreadable)).is_some(),
            "{cut} バイト: {notes:?}"
        );
    }
}

/// 透明の背景に、赤（不透明）と青（半透明）を重ねた PSD（統合透明度つき）。
fn transparent_background(compression: u16) -> Psd {
    let mut blue = L::new("青", (3, 3, 7, 7), [0, 0, 255, 255]);
    blue.opacity = 128;
    let mut psd = Psd::new(8, 8, vec![red("赤", (1, 1, 5, 5)), blue]);
    psd.merged_alpha = true;
    psd.merged_compression = compression;
    psd
}

#[test]
fn a_transparent_merged_image_is_compared_with_its_alpha_plane_in_every_compression() {
    for compression in 0..=3 {
        let bytes = transparent_background(compression).build();
        // 原本を保つ読みも、統合透明度の宣言どおりに読む
        assert_ne!(
            psd::read(&bytes, &Limits::default()).unwrap().mode(),
            CompatibilityMode::Rejected,
            "圧縮 {compression}"
        );
        let (d, notes) = imported(&bytes);
        assert_eq!(d.layers().len(), 2);
        // 統合画像は透明度の面まで読めて、差は無い（照合を省略もしない。RGB だけの照合に落ちていない）
        assert!(
            !notes.iter().any(|n| matches!(
                n.feature,
                ImportFeature::CompositeDiffers { .. } | ImportFeature::CompositeUnchecked(_) | ImportFeature::ExtraChannel
            )),
            "圧縮 {compression}: {notes:?}"
        );
    }
    // 選択範囲のアルファチャンネルがもう 1 つあっても、透明度の面だけが照合に入る（追加の面は飛ばす）
    for compression in 0..=3 {
        let mut psd = transparent_background(compression);
        psd.extra_channels = 1;
        let (_, notes) = imported(&psd.build());
        assert!(
            !notes.iter().any(|n| matches!(n.feature, ImportFeature::CompositeDiffers { .. } | ImportFeature::CompositeUnchecked(_))),
            "圧縮 {compression}・追加チャンネル 1: {notes:?}"
        );
        assert_eq!(note(&notes, ImportFeature::ExtraChannel).unwrap().action, ImportAction::Ignored);
    }
}

#[test]
fn a_wrong_alpha_plane_in_the_merged_image_is_reported_with_the_largest_difference() {
    // 色の面は正しく、透明度の面だけを不透明（255）にした統合画像。差は透明度の面にしか無いので、
    // 4 面目を比べていなければ見逃す。差のある画素は、透明な 36 画素と半透明だけの 12 画素の計 48
    for compression in 0..=3 {
        let mut psd = transparent_background(compression);
        psd.merged_alpha_override = Some(255);
        let (_, notes) = imported(&psd.build());
        let n = notes
            .iter()
            .find_map(|n| match n.feature {
                ImportFeature::CompositeDiffers { max_diff, differing, total } => Some((n.action, max_diff, differing, total)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("圧縮 {compression}: {notes:?}"));
        assert_eq!(n, (ImportAction::Changed, 255, 48, 64), "圧縮 {compression}");
    }
    // 透明度を持たない宣言（層の数が正）の 4 チャンネルは、透明度として比べない（追加のチャンネルとして飛ばす）
    let mut psd = transparent_background(0);
    psd.merged_alpha = false;
    psd.extra_channels = 1;
    let (_, notes) = imported(&psd.build());
    assert!(note(&notes, ImportFeature::ExtraChannel).is_some(), "{notes:?}");
    assert!(
        !notes.iter().any(|n| matches!(n.feature, ImportFeature::CompositeDiffers { .. })),
        "{notes:?}"
    );
}

// ───────── 断るもの・上限 ─────────

#[test]
fn what_cannot_be_imported_is_refused_with_a_reason() {
    let ok = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]).build();
    // PSB
    let mut psb = ok.clone();
    psb[4..6].copy_from_slice(&be16(2));
    assert_eq!(refused(&psb, 256 * MIB), CopyRefusal::LargeDocument);
    // 16 bit・CMYK
    let mut deep = ok.clone();
    deep[22..24].copy_from_slice(&be16(16));
    assert_eq!(refused(&deep, 256 * MIB), CopyRefusal::ColorFormat { depth: 16, mode: 3 });
    let mut cmyk = ok.clone();
    cmyk[24..26].copy_from_slice(&be16(4));
    assert_eq!(refused(&cmyk, 256 * MIB), CopyRefusal::ColorFormat { depth: 8, mode: 4 });
    // 壊れている・途中で切れている・PSD ではない
    assert!(matches!(refused(&ok[..ok.len() / 2], 256 * MIB), CopyRefusal::Malformed(_)));
    assert!(matches!(refused(&ok[..10], 256 * MIB), CopyRefusal::Malformed(_)));
    assert!(matches!(refused(b"not a psd at all, definitely not a psd", 256 * MIB), CopyRefusal::Malformed(_)));
    // レイヤーが無い
    let mut none = Psd::new(4, 4, vec![]);
    none.layers.clear();
    assert_eq!(refused(&none.build(), 256 * MIB), CopyRefusal::NoLayers);
}

#[test]
fn the_layer_count_the_canvas_and_the_pixels_follow_the_source_budget() {
    // 層の数: 予算 256 MiB では 256 枚まで（原本を保つ読みの既定と同じ）。300 枚の PSD は、予算を上げれば取り込める
    let layers: Vec<L> = (0..300)
        .map(|i| L::new(&format!("L{i}"), (0, 0, 1, 1), [i as u8, 0, 0, 255]).id(i + 1))
        .collect();
    let bytes = Psd::new(2, 2, layers).build();
    assert_eq!(psd::read(&bytes, &Limits::default()).unwrap().mode(), CompatibilityMode::Rejected);
    assert_eq!(
        refused(&bytes, 256 * MIB),
        CopyRefusal::TooManyLayers { count: 300, limit: 256 }
    );
    assert!(CopyRefusal::TooManyLayers { count: 300, limit: 256 }.raised_by_budget());
    match run(&bytes, 512 * MIB) {
        CopyOutcome::Imported(i) => assert_eq!(i.document.layers().len(), 300),
        CopyOutcome::Refused(why) => panic!("{}", why.message()),
    }
    // 画布: 1 枚ぶんの画素が予算を超える
    let big = Psd::new(512, 512, vec![red("a", (0, 0, 1, 1))]).build();
    assert_eq!(refused(&big, 512 * 1024), CopyRefusal::CanvasTooLarge { width: 512, height: 512 });
    // 層の画素: 文書の予算を超える層で止まる（層の名前つき）
    let layers: Vec<L> = (0..3)
        .map(|i| {
            let mut l = L::new(&format!("面{i}"), (0, 0, 256, 256), [0; 4]).id(i + 1);
            // 一様でない画素（一様なタイルは小さく持つので、予算に数えるには画素を散らす）
            for (k, px) in l.rgba.chunks_mut(4).enumerate() {
                px.copy_from_slice(&[(k * 7 + i as usize) as u8, (k / 256) as u8, (k % 251) as u8, 255]);
            }
            l
        })
        .collect();
    let bytes = Psd::new(256, 256, layers).build();
    // 256×256×4 = 256 KiB の層が 3 枚。予算 600 KiB には 2 枚まで入る
    assert_eq!(
        refused(&bytes, 600 * 1024),
        CopyRefusal::BudgetExceeded { layer: "面2".into() }
    );
    assert!(matches!(run(&bytes, 4 * MIB), CopyOutcome::Imported(_)));
    // 1 枚の層が予算に入らない
    let one = Psd::new(256, 256, vec![L::new("巨大", (0, 0, 256, 256), [1, 2, 3, 255])]).build();
    assert!(matches!(
        refused(&one, 256 * 1024 - 1),
        CopyRefusal::CanvasTooLarge { .. }
    ));
}

#[test]
fn a_layers_extra_data_follows_the_source_budget_and_can_be_raised() {
    // 層 1 枚の付加情報（効果・スマートオブジェクトの中身など）が予算より大きい PSD は、固定の上限でなく予算で断る
    let big = vec![7u8; 2 * MIB as usize];
    let bytes = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4)), red("b", (0, 0, 4, 4)).tag(b"SoLd", &big)]).build();
    let why = refused(&bytes, MIB);
    assert_eq!(why, CopyRefusal::LayerDataTooLarge { layer: "#2".into() });
    // 設定の予算を上げれば取り込める（行のツールチップが「上げられる」と言うので、予算で解ける理由であること）
    assert!(why.raised_by_budget());
    match run(&bytes, 4 * MIB) {
        CopyOutcome::Imported(i) => {
            assert_eq!(i.document.layers().len(), 2);
            let n = i.notes.iter().find(|n| n.feature == ImportFeature::SmartObject).unwrap();
            assert_eq!((n.action, n.layers.clone()), (ImportAction::Dropped, vec!["b".to_string()]));
        }
        CopyOutcome::Refused(why) => panic!("{}", why.message()),
    }
    // 付加情報が層の区間を超える PSD は、大きさでなく壊れているとして断る
    let mut cut = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4))]).build();
    let name = cut.windows(8).position(|w| w == [1, b'a', 0, 0, b'8', b'B', b'I', b'M']).unwrap();
    // 付加情報の長さ（名前の手前、マスク長 4 + 範囲長 4 の前）を大きくする
    let at = name - 8 - 4;
    cut[at..at + 4].copy_from_slice(&be32(0x7fff_0000));
    assert!(matches!(refused(&cut, 256 * MIB), CopyRefusal::Malformed(_)));
}

#[test]
fn a_cancel_flag_stops_the_import_without_a_result() {
    let layers: Vec<L> = (0..4).map(|i| L::new(&format!("L{i}"), (0, 0, 1, 1), [1, 2, 3, 255]).id(i + 1)).collect();
    let bytes = Psd::new(2, 2, layers).build();
    let flag = AtomicBool::new(true);
    let e = psd::import_copy(
        &mut Cursor::new(bytes.clone()),
        &CopyOptions { source_budget: 256 * MIB, cancel: Some(&flag) },
    )
    .err()
    .expect("取り消すと結果は無い");
    assert!(matches!(e, Error::Core(yolu_core::CoreError::Cancelled)), "{e}");
    flag.store(false, Ordering::Relaxed);
    assert!(matches!(run_with(&bytes, 256 * MIB, Some(&flag)), CopyOutcome::Imported(_)));
}

// ───────── 原本を持たない・流して読む ─────────

/// 長い 0 の区間を持つ仮想のファイル（本物の 200 MiB を作らずに、大きな PSD を試す）。読んだバイト数を数える。
struct Sparse {
    /// （位置, 中身）の並び。中身の無い区間は 0。
    parts: Vec<(u64, Option<Vec<u8>>, u64)>,
    pos: u64,
    len: u64,
    read: u64,
}
impl Sparse {
    /// `before` の次に `hole` バイトの 0 の画像リソースを挟んだ PSD。
    fn with_resource(psd: &[u8], hole: u64) -> Sparse {
        // ヘッダー（26）+ 色モード（4）= 30 バイトの後が画像リソース: 長さ 4 バイト
        let head = psd[..30].to_vec();
        let rest = psd[34..].to_vec(); // 元の（空の）画像リソースの長さを飛ばした残り
        let block_head = {
            let mut b = Vec::new();
            b.extend(b"8BIM");
            b.extend(be16(1006));
            b.extend([0, 0]);
            b.extend(be32(hole as u32));
            b
        };
        let res_len = block_head.len() as u64 + hole;
        let mut first = head;
        first.extend(be32(res_len as u32));
        first.extend(block_head);
        let len = first.len() as u64 + hole + rest.len() as u64;
        let first_len = first.len() as u64;
        Sparse {
            parts: vec![(0, Some(first), first_len), (first_len, None, hole), (first_len + hole, Some(rest), len - first_len - hole)],
            pos: 0,
            len,
            read: 0,
        }
    }
}
impl Read for Sparse {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.len {
            return Ok(0);
        }
        let (start, bytes, size) = self.parts.iter().find(|p| self.pos >= p.0 && self.pos < p.0 + p.2).unwrap().clone();
        let offset = self.pos - start;
        let n = buf.len().min((size - offset) as usize);
        match bytes {
            Some(b) => buf[..n].copy_from_slice(&b[offset as usize..offset as usize + n]),
            None => buf[..n].fill(0),
        }
        self.pos += n as u64;
        self.read += n as u64;
        Ok(n)
    }
}
impl Seek for Sparse {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        self.pos = match to {
            SeekFrom::Start(p) => p,
            SeekFrom::Current(d) => (self.pos as i64 + d) as u64,
            SeekFrom::End(d) => (self.len as i64 + d) as u64,
        };
        Ok(self.pos)
    }
}

#[test]
fn a_file_over_the_preserving_reader_limit_imports_without_reading_it_all() {
    let base = Psd::new(4, 4, vec![red("a", (0, 0, 4, 4)), L::new("b", (1, 1, 3, 3), [0, 255, 0, 255])]).build();
    // 200 MiB の画像リソース。原本を保つ読みの上限（128 MiB）を超える
    let mut file = Sparse::with_resource(&base, 200 * MIB);
    assert!(file.len > Limits::default().max_source_bytes as u64);
    let r = psd::read_stream(&mut Sparse::with_resource(&base, 200 * MIB), &Limits::default()).unwrap();
    assert_eq!(r.mode(), CompatibilityMode::Rejected);
    let outcome = psd::import_copy(&mut file, &CopyOptions { source_budget: 256 * MIB, cancel: None }).unwrap();
    let CopyOutcome::Imported(i) = outcome else {
        panic!("取り込めない")
    };
    assert_eq!(i.document.layers().len(), 2);
    assert_eq!(note(&i.notes, ImportFeature::ImageResources).unwrap().count, 1);
    // 200 MiB は読み飛ばした（読んだのは、層の記録と画素と統合画像だけ）
    assert!(file.read < MIB, "読んだバイト数 {}", file.read);
}

// ───────── 取り込んだ写しの書き出しは新しい PSD ─────────

#[test]
fn a_copy_exports_as_a_new_psd_and_the_imported_document_is_an_ordinary_one() {
    // 原本を保つ読みで編集できる PSD（書き出しの形）も、写しとして同じ文書になる
    let doc = Document {
        width: 4,
        height: 4,
        layers: vec![
            Layer {
                id: 11,
                name: "下".into(),
                width: 4,
                height: 4,
                pixels_rgba: [255, 0, 0, 255].repeat(16),
                ..Layer::default()
            },
            Layer {
                id: 12,
                name: "反転".into(),
                kind: LayerKind::Adjustment(Adjustment::Invert),
                blend_mode: BlendMode::Normal,
                opacity: 128,
                ..Layer::default()
            },
            Layer {
                id: 13,
                name: "マスク付き".into(),
                left: 1,
                top: 1,
                width: 2,
                height: 2,
                pixels_rgba: [0, 0, 255, 255].repeat(4),
                mask: Some(Mask {
                    left: 0,
                    top: 0,
                    width: 4,
                    height: 4,
                    default_color: 255,
                    enabled: true,
                    density: 255,
                    pixels: (0..16).map(|i| (i * 16) as u8).collect(),
                }),
                ..Layer::default()
            },
        ],
        composite_rgba: None,
    };
    let bytes = psd::write(&doc, &Limits::default()).unwrap();
    let strict = psd::read(&bytes, &Limits::default()).unwrap();
    assert_eq!(strict.mode(), CompatibilityMode::EditableRaster);
    let via_strict = strict.to_core().unwrap();
    let (copy, notes) = imported(&bytes);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(composite(&copy), composite(&via_strict));
    let ids = |d: &yolu_core::Document| d.layers().iter().map(|l| l.id().0 >> 96).collect::<Vec<_>>();
    assert_eq!(ids(&copy), ids(&via_strict));
    // 書き出し直すと、元の PSD とは別の、新しい PSD が書ける（原本へ書き戻す道は無い）
    let projected = psd::Document::from_core(&copy).unwrap();
    let again = psd::write(&projected, &Limits::default()).unwrap();
    let back = psd::read(&again, &Limits::default()).unwrap();
    assert_eq!(back.mode(), CompatibilityMode::EditableRaster);
    assert_eq!(composite(&back.to_core().unwrap()), composite(&copy));
}
