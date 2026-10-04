//! 層の画素のコピー・カット・結合してコピー・ペースト・置き換えと、まとめ（batch）を、実 C# Core の出力と全バイトで照らす。
//! 台本・人工の文書・出力の並びは tools/csharp-golden/ClipboardGolden.cs と対。C# は事例ごとの出力の SHA-256 を index.txt に
//! 書く（`tools/csharp-golden/run.sh clipboard`）。食い違ったときは `CLIPBOARD_GOLDEN_DUMP=フォルダ` で Rust の出力を書き出し、
//! C# の `GOLDEN_FULL=フォルダ` の出力と比べる。並列度 1 と 4 の両方で同じ結果になる。

// 画素の格子を (x, y) の添字で見比べる試験なので、添字の範囲の繰り返しの方が読みやすい。
#![allow(clippy::needless_range_loop)]

use sha2::{Digest, Sha256};
use yolu_core::*;

const MAX: u64 = u64::MAX;

// ───────── 人工の文書（DocOpsGolden.cs と同じ） ─────────
fn pattern(x: u32, y: u32, c: u32, seed: u32) -> Rgba8 {
    let n = x * 17 + y * 31 + c * 13 + seed * 7;
    Rgba8::new(
        (n * 3 + 1) as u8,
        (n * 5 + 2) as u8,
        (n * 7 + 3) as u8,
        if n.is_multiple_of(5) {
            0
        } else if n % 5 == 1 {
            255
        } else {
            (n % 256) as u8
        },
    )
}
fn make(seed: u32) -> Document {
    let mut d = Document::with_tile_size(17, 13, [4, 8, 16][seed as usize % 3]).unwrap();
    for l in 0..2 {
        let a = d.add_layer(&format!("layer{l}")).unwrap();
        for c in Channel::ALL {
            d.set_channel_enabled(a, c, true).unwrap();
            for y in 0..13 {
                for x in 0..17 {
                    if !(x + y + seed + l).is_multiple_of(4) {
                        d.set_channel_pixel(a, c, x, y, pattern(x, y, c.index() as u32, seed + l))
                            .unwrap();
                    }
                }
            }
        }
        d.add_layer_mask(a).unwrap();
        for y in 0..13 {
            for x in 0..17 {
                if (x + y) % 5 == 0 {
                    d.set_mask_pixel(a, x, y, (x * 11 + y * 7) as u8).unwrap();
                }
            }
        }
    }
    d.clear_history().unwrap();
    d
}
/// 貼り先の文書（ClipboardGolden.cs の MakeTarget）。
fn make_target(kind: u32, seed: u32, grouped: bool) -> (Document, LayerId) {
    let (w, h) = [(17, 13), (9, 7), (25, 21), (17, 13)][kind as usize];
    let tile = if kind == 0 || kind == 3 {
        [4, 8, 16][seed as usize % 3]
    } else {
        8
    };
    let mut d = Document::with_tile_size(w, h, tile).unwrap();
    let base = d.add_layer("base").unwrap();
    for y in 0..h {
        for x in 0..w {
            if (x + y) % 3 != 0 {
                d.set_pixel(base, x, y, pattern(x, y, 0, seed + 9)).unwrap();
            }
        }
    }
    if grouped {
        d.group_layers(&[base], "g").unwrap();
    }
    if kind == 3 {
        let s = SelectionMask::rectangle(&d, 2, 2, 11, 9);
        d.set_selection(Some(s)).unwrap();
    }
    d.clear_history().unwrap();
    (d, base)
}

// ───────── 出力（.NET の BinaryWriter と同じ並び） ─────────
fn i32b(out: &mut Vec<u8>, n: i32) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn i64b(out: &mut Vec<u8>, n: i64) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn f64b(out: &mut Vec<u8>, n: f64) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn string_b(out: &mut Vec<u8>, s: &str) {
    assert!(s.len() < 128);
    out.push(s.len() as u8);
    out.extend_from_slice(s.as_bytes());
}
fn snapshot(out: &mut Vec<u8>, d: &Document) {
    i32b(out, d.width() as i32);
    i32b(out, d.height() as i32);
    i32b(out, d.layers().len() as i32);
    f64b(out, d.normal_settings().strength());
    out.push(d.selection().is_some() as u8);
    if let Some(m) = d.selection() {
        out.extend(m.to_canvas_bytes());
    }
    for l in d.layers() {
        i32b(out, l.kind() as i32);
        i32b(
            out,
            l.parent().map_or(-1, |p| d.layer_index(p).unwrap() as i32),
        );
        out.push(l.visible() as u8);
        f64b(out, l.opacity());
        i32b(out, l.blend_mode() as i32);
        out.push(l.clipping() as u8);
        i32b(out, l.locks().bits() as i32);
        for c in Channel::ALL {
            out.push(l.is_channel_enabled(c) as u8);
            out.push(l.surface(c).is_some() as u8);
            i32b(out, l.blend_mode_in(c) as i32);
            f64b(out, l.opacity_in(c));
            for y in 0..d.height() {
                for x in 0..d.width() {
                    out.extend_from_slice(&l.pixel(c, x, y).unwrap().to_array());
                }
            }
        }
        out.push(l.mask().is_some() as u8);
        if let Some(m) = l.mask() {
            out.push(m.enabled() as u8);
            out.push(m.inverted() as u8);
            f64b(out, m.density());
            for y in 0..d.height() {
                for x in 0..d.width() {
                    out.push(m.surface().pixel(x, y).unwrap().a);
                }
            }
        }
    }
    for c in Channel::ALL {
        out.extend(d.composite_channel(c, d.bounds()).unwrap());
    }
}
fn write_clip(out: &mut Vec<u8>, c: &PixelClipboard) {
    let r = c.rect();
    let (dw, dh) = c.document_size();
    for v in [r.x, r.y, r.width, r.height, dw, dh] {
        i32b(out, v as i32);
    }
    i32b(
        out,
        match c.source() {
            ClipboardSource::Layer => 0,
            ClipboardSource::Mask => 1,
            ClipboardSource::Composite => 2,
            ClipboardSource::External => panic!("C# に無い出どころ"),
        },
    );
    i32b(out, c.channel().index() as i32);
    out.extend_from_slice(c.pixels());
}
/// 断りの型（ClipboardGolden.cs の Fail）: 1 ロック、2 層の操作の断り、3 そのほかの InvalidOperation、4 引数の誤り。
fn fail(out: &mut Vec<u8>, d: &Document, e: CoreError) {
    match e {
        CoreError::LayerLocked {
            layer,
            holder,
            lock,
        } => {
            out.push(1);
            i32b(out, d.layer_index(layer).unwrap() as i32);
            i32b(out, d.layer_index(holder).unwrap() as i32);
            i32b(out, lock.bits() as i32);
        }
        CoreError::Clipboard(r) => {
            out.push(2);
            let (name, bytes, limit) = match r {
                ClipboardRefusal::NoPixels => ("NoPixels", 0, 0),
                ClipboardRefusal::NothingToCopy { .. } => ("NothingToCopy", 0, 0),
                ClipboardRefusal::TooLarge { bytes, limit } => ("ClipboardTooLarge", bytes, limit),
                ClipboardRefusal::OperationBudget { bytes, limit } => {
                    ("OperationBudget", bytes, limit)
                }
                ClipboardRefusal::NotPaintLayer => ("NotPaintLayer", 0, 0),
            };
            string_b(out, name);
            i64b(out, bytes as i64);
            i64b(out, limit as i64);
        }
        CoreError::InvalidArgument(_) => out.push(4),
        CoreError::Unsupported(_)
        | CoreError::SourceBudgetExceeded
        | CoreError::BatchActive
        | CoreError::StrokeActive => out.push(3),
        other => panic!("想定外の失敗 {other:?}"),
    }
}
fn after(out: &mut Vec<u8>, d: &mut Document, changed: bool) {
    out.extend_from_slice(&d.history_bytes().to_le_bytes());
    snapshot(out, d);
    if changed {
        d.undo().unwrap();
        out.extend_from_slice(&d.history_bytes().to_le_bytes());
        snapshot(out, d);
        d.redo().unwrap();
        out.extend_from_slice(&d.history_bytes().to_le_bytes());
        snapshot(out, d);
    }
}
fn apply_selection(d: &mut Document, variant: u32) {
    match variant {
        1 => {
            let s = SelectionMask::rectangle(d, 3, 2, 12, 9);
            d.set_selection(Some(s)).unwrap();
        }
        2 => {
            // 量が (x*17+y*29)%256 の中ほど
            let ts = d.tile_size();
            let mut tiles = Vec::new();
            for coord in d.canvas_tiles() {
                let mut v = vec![0; (ts * ts) as usize];
                for y in 0..ts.min(d.height() - coord.y * ts) {
                    for x in 0..ts.min(d.width() - coord.x * ts) {
                        let (px, py) = (coord.x * ts + x, coord.y * ts + y);
                        if (3..14).contains(&px) && (2..11).contains(&py) {
                            v[(y * ts + x) as usize] = ((px * 17 + py * 29) % 256) as u8;
                        }
                    }
                }
                if v.iter().any(|&a| a != 0) {
                    tiles.push((coord, v));
                }
            }
            let s = SelectionMask::from_amount_tiles(d.width(), d.height(), ts, tiles).unwrap();
            d.set_selection(Some(s)).unwrap();
        }
        3 => {
            let s = SelectionMask::rectangle(d, 15, 11, 17, 13);
            d.set_selection(Some(s)).unwrap();
        }
        _ => {}
    }
}
fn chan(n: u32) -> Channel {
    Channel::from_index(n as usize).unwrap()
}

// ───────── 事例 ─────────
fn copy(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let layer = d.layers()[n as usize % 2].id();
    apply_selection(&mut d, (n / 12) % 4);
    d.clear_history().unwrap();
    match d.copy_pixels(layer, chan((n / 2) % 6), false, MAX) {
        Ok(c) => {
            o.push(0);
            write_clip(o, &c);
        }
        Err(e) => fail(o, &d, e),
    }
    after(o, &mut d, false);
}
fn mask_copy(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let layer = d.layers()[n as usize % 2].id();
    apply_selection(&mut d, (n / 2) % 4);
    d.clear_history().unwrap();
    match d.copy_pixels(layer, Channel::Color, true, MAX) {
        Ok(c) => {
            o.push(0);
            write_clip(o, &c);
        }
        Err(e) => fail(o, &d, e),
    }
    after(o, &mut d, false);
}
/// 写せる大きさの上限（上限を超えた時点の矩形の大きさ・タイルを読む順が出力に出る）。0〜23 は層、24〜47 は結合。
fn limit(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let layer = d.layers()[n as usize % 2].id();
    let limit = [100, 300, 700, 1500][(n as usize / 2) % 4];
    apply_selection(&mut d, (n / 8) % 3);
    d.clear_history().unwrap();
    let result = if n < 24 {
        d.copy_pixels(layer, Channel::Color, false, limit)
    } else {
        d.copy_merged(Channel::Color, limit)
    };
    match result {
        Ok(c) => {
            o.push(0);
            write_clip(o, &c);
        }
        Err(e) => fail(o, &d, e),
    }
    after(o, &mut d, false);
}
fn merged(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let (a, b) = (d.layers()[0].id(), d.layers()[1].id());
    match n % 8 {
        0 => d.set_layer_clipping(b, true).unwrap(),
        1 => {
            d.set_layer_opacity(a, 0.4, false).unwrap();
            d.set_layer_blend_mode(a, BlendMode::Multiply).unwrap();
        }
        2 => {
            let bg = d
                .add_fill_layer(
                    "background",
                    &[(Channel::Color, Rgba8::new(100, 200, 70, 255))],
                    None,
                )
                .unwrap();
            d.move_layer(bg, 0).unwrap();
        }
        3 => d.set_layer_blend_mode(b, BlendMode::Screen).unwrap(),
        4 => d.set_channel_enabled(a, Channel::Normal, false).unwrap(),
        5 => d.set_channel_enabled(b, Channel::Metallic, false).unwrap(),
        6 => {
            d.set_layer_mask_inverted(a, true).unwrap();
            d.set_layer_mask_density(a, 0.35, false).unwrap();
        }
        _ => {
            d.set_channel_blend(
                a,
                Channel::Color,
                ChannelBlend::new(Some(BlendMode::Multiply), Some(0.65)),
                false,
            )
            .unwrap();
            d.set_layer_opacity(b, 0.55, false).unwrap();
        }
    }
    let channel = if (n / 8).is_multiple_of(2) {
        Channel::Color
    } else {
        Channel::Normal
    };
    apply_selection(&mut d, (n / 16) % 4);
    d.clear_history().unwrap();
    match d.copy_merged(channel, MAX) {
        Ok(c) => {
            o.push(0);
            write_clip(o, &c);
        }
        Err(e) => fail(o, &d, e),
    }
    after(o, &mut d, false);
}
fn cut(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let (a, b) = (d.layers()[0].id(), d.layers()[1].id());
    let (mut layer, mut channel, mut mask) = (a, Channel::Color, false);
    match n % 14 {
        1 => {
            layer = b;
            channel = Channel::Roughness;
        }
        2 => channel = Channel::Normal,
        3 => d.set_layer_locks(a, LayerLocks::TRANSPARENCY).unwrap(),
        4 => d.set_layer_locks(a, LayerLocks::PIXELS).unwrap(),
        5 => d.set_layer_locks(a, LayerLocks::ALL).unwrap(),
        6 => {
            let g = d.group_layers(&[a], "g").unwrap();
            d.set_layer_locks(g, LayerLocks::PIXELS).unwrap();
        }
        7 => mask = true,
        8 => {
            mask = true;
            d.set_layer_locks(a, LayerLocks::PIXELS).unwrap();
        }
        9 => {
            mask = true;
            d.set_layer_locks(a, LayerLocks::ALL).unwrap();
        }
        10 => {
            layer = d
                .add_fill_layer(
                    "fill",
                    &[(Channel::Color, Rgba8::new(10, 200, 30, 255))],
                    None,
                )
                .unwrap();
        }
        11 => layer = d.add_group("empty", None).unwrap(),
        12 => {
            d.set_channel_enabled(a, Channel::Roughness, false).unwrap();
            channel = Channel::Roughness;
        }
        13 => d.set_layer_locks(a, LayerLocks::POSITION).unwrap(),
        _ => {}
    }
    apply_selection(&mut d, (n / 14) % 4);
    d.clear_history().unwrap();
    let mut ok = false;
    match d.cut_pixels(layer, channel, mask, MAX) {
        Ok(c) => {
            o.push(0);
            write_clip(o, &c);
            ok = true;
        }
        Err(e) => fail(o, &d, e),
    }
    let changed = ok && d.undo_count() > 0;
    after(o, &mut d, changed);
}
fn paste(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut source = make(seed);
    let channel = [Channel::Color, Channel::Roughness, Channel::Normal][n as usize % 3];
    apply_selection(&mut source, (n / 3) % 3);
    source.clear_history().unwrap();
    let clip = match source.copy_pixels(source.layers()[0].id(), channel, false, MAX) {
        Ok(c) => c,
        Err(e) => {
            fail(o, &source, e);
            after(o, &mut source, false);
            return;
        }
    };
    let (kind, above, budget) = ((n / 9) % 4, (n / 36) % 3, (n / 108) % 3);
    let (mut t, base) = make_target(kind, seed, above == 2);
    if budget == 1 {
        t.set_stroke_budget_bytes(100).unwrap();
    }
    if budget == 2 {
        let bytes = t.allocated_bytes() + 8;
        t.set_source_budget_bytes(bytes).unwrap();
    }
    let above = (above != 0).then_some(base);
    let mut ok = false;
    match t.paste_as_layer(&clip, channel, Some("pasted"), above) {
        Ok(r) => {
            ok = true;
            o.push(0);
            i32b(o, r.x);
            i32b(o, r.y);
            o.push(r.centered as u8);
            i64b(o, r.clipped_pixels as i64);
            i32b(
                o,
                t.layers().iter().position(|l| l.id() == r.layer).unwrap() as i32,
            );
        }
        Err(e) => fail(o, &t, e),
    }
    after(o, &mut t, ok);
}
fn image(w: u32, h: u32, seed: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for y in 0..h {
        for x in 0..w {
            bytes.extend_from_slice(&pattern(x, y, 5, seed + 3).to_array());
        }
    }
    bytes
}
fn replace(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let a = d.layers()[0].id();
    let channel = if (n / 32).is_multiple_of(2) {
        Channel::Color
    } else {
        Channel::Normal
    };
    match (n / 8) % 4 {
        1 => d.set_layer_locks(a, LayerLocks::TRANSPARENCY).unwrap(),
        2 => d.set_layer_locks(a, LayerLocks::PIXELS).unwrap(),
        3 => d.set_layer_locks(a, LayerLocks::ALL).unwrap(),
        _ => {}
    }
    apply_selection(&mut d, n % 4);
    d.clear_history().unwrap();
    let mut ok = false;
    match d.replace_pixels(a, channel, &image(17, 13, seed), (n / 4).is_multiple_of(2)) {
        Ok(changed) => {
            o.push(0);
            o.push(changed as u8);
            ok = changed;
        }
        Err(e) => fail(o, &d, e),
    }
    after(o, &mut d, ok);
}
fn batch_case(n: u32, seed: u32, o: &mut Vec<u8>) {
    let mut d = make(seed);
    let (a, b) = (d.layers()[0].id(), d.layers()[1].id());
    let mut changed = false;
    if n == 2 {
        apply_selection(&mut d, 1);
    }
    d.clear_history().unwrap();
    if n == 1 {
        d.set_layer_opacity(a, 0.7, false).unwrap();
        d.undo().unwrap(); // 1 段のやり直しを残す
    }
    let history = d.history_bytes();
    let result = match n {
        0 => d.batch(|d| {
            d.fill(
                a,
                Channel::Color,
                Rgba8::new(10, 200, 30, 200),
                0.8,
                None,
                false,
            )?;
            d.set_layer_opacity(b, 0.5, false)?;
            let l = d.add_layer("added")?;
            d.set_channel_enabled(l, Channel::Roughness, true)?;
            Ok(())
        }),
        1 => d.batch(|d| {
            d.fill(
                a,
                Channel::Color,
                Rgba8::new(0, 255, 0, 255),
                1.0,
                None,
                false,
            )?;
            d.add_layer("doomed")?;
            Err(CoreError::Unsupported("boom"))
        }),
        2 => d.batch(|d| {
            let clip = d.cut_pixels(a, Channel::Color, false, MAX)?;
            let r = d.paste_as_layer(&clip, Channel::Color, Some("moved"), Some(a))?;
            d.set_layer_opacity(r.layer, 0.5, false)
        }),
        3 => d.batch(|d| {
            d.set_layer_opacity(a, 0.3, false)?;
            d.undo().map(drop)
        }),
        _ => d.batch(|d| {
            d.set_layer_opacity(a, 0.3, false)?;
            d.batch(|_| Ok(()))
        }),
    };
    match result {
        Ok(()) => {
            o.push(0);
            changed = n == 0 || n == 2;
        }
        Err(e) => fail(o, &d, e),
    }
    i32b(o, d.undo_count() as i32);
    o.push(d.can_redo() as u8);
    o.extend_from_slice(&history.to_le_bytes());
    after(o, &mut d, changed);
    if n == 1 {
        d.redo().unwrap();
        o.extend_from_slice(&d.history_bytes().to_le_bytes());
        snapshot(o, &d);
    }
}
fn run(op: &str, n: u32, seed: u32) -> Vec<u8> {
    let mut o = Vec::new();
    match op {
        "clipcopy" => copy(n, seed, &mut o),
        "clipmask" => mask_copy(n, seed, &mut o),
        "cliplimit" => limit(n, seed, &mut o),
        "clipmerged" => merged(n, seed, &mut o),
        "clipcut" => cut(n, seed, &mut o),
        "clippaste" => paste(n, seed, &mut o),
        "clipreplace" => replace(n, seed, &mut o),
        "clipbatch" => batch_case(n, seed, &mut o),
        _ => panic!("未知の操作 {op}"),
    }
    o
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn csharp_clipboard_all_bytes_and_parallelism() {
    let dump = std::env::var("CLIPBOARD_GOLDEN_DUMP").ok();
    let lines: Vec<_> = include_str!("golden/clipboard/index.txt")
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .collect();
    assert!(lines.len() > 800, "{}", lines.len());
    let mut ops = std::collections::BTreeMap::<&str, usize>::new();
    for degree in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap();
        let mut bad = Vec::new();
        pool.install(|| {
            for line in &lines {
                let p: Vec<_> = line.split_whitespace().collect();
                let actual = run(p[1], p[2].parse().unwrap(), p[3].parse().unwrap());
                if let Some(dir) = &dump {
                    std::fs::create_dir_all(dir).unwrap();
                    std::fs::write(format!("{dir}/{}.bin", p[0]), &actual).unwrap();
                }
                let digest = hex(&Sha256::digest(&actual));
                if digest != p[4] || actual.len().to_string() != p[5] {
                    bad.push(format!("{} ({} bytes, C# {})", p[0], actual.len(), p[5]));
                }
                *ops.entry(p[1]).or_default() += 1;
            }
        });
        assert!(
            bad.is_empty(),
            "degree={degree} C# と食い違う {} 件: {:?}",
            bad.len(),
            &bad[..bad.len().min(12)]
        );
    }
    assert_eq!(ops.len(), 8, "{ops:?}");
}
