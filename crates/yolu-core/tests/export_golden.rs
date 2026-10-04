//! 書き出し（テンプレートとパディング）を Unity 版の C# の Core（ExportTemplates・TexturePadding）の出力と全バイトで照らし合わせる。
//! 台本は tests/golden/export/cases.txt、正解は同じフォルダの index.txt と <事例>.bin（tools/csharp-golden/run.sh export で作る）。
//! 乱数・台本の読み方・出力の書き方は tools/csharp-golden/ExportGolden.cs と揃えてある（片方を変えたら両方を変える）。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use yolu_core::export::{build, should_write, ExportTemplate};
use yolu_core::glam::DVec2;
use yolu_core::padding::{coverage, dilate, Reach, Tuning};
use yolu_core::{
    AdjustmentSettings, BlendMode, Channel, Document, HeightEdgeMode, LayerId, NormalSettings,
    NormalYDirection, Rgba8, TileCoord,
};

struct SplitMix(u64);
impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn u01(&mut self) -> f64 {
        (self.next() >> 11) as f64 * (1.0 / 9007199254740992.0)
    }
    fn channel(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 => 0,
            1 => 255,
            _ => (r >> 8) as u8,
        }
    }
    fn alpha(&mut self) -> u8 {
        let r = self.next();
        match r & 7 {
            0 | 1 => 0,
            2 | 3 => 255,
            _ => (r >> 8) as u8,
        }
    }
    fn rgba(&mut self) -> Rgba8 {
        let r = self.channel();
        let g = self.channel();
        let b = self.channel();
        let a = self.alpha();
        Rgba8::new(r, g, b, a)
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/export")
}

fn int(s: &str) -> i64 {
    s.parse().unwrap_or_else(|_| panic!("整数: {s}"))
}
fn dbl(s: &str) -> f64 {
    match s {
        "nan" => f64::NAN,
        "inf" => f64::INFINITY,
        "-inf" => f64::NEG_INFINITY,
        _ => s.parse().unwrap_or_else(|_| panic!("数: {s}")),
    }
}
fn color(s: &str) -> Rgba8 {
    let p: Vec<u8> = s.split(',').map(|v| v.parse().expect("色")).collect();
    Rgba8::new(p[0], p[1], p[2], p[3])
}
fn mode(s: &str) -> BlendMode {
    BlendMode::from_name(s).unwrap_or_else(|| panic!("モード: {s}"))
}
fn channel(s: &str) -> Channel {
    Channel::from_standard_name(s).unwrap_or_else(|| panic!("チャンネル: {s}"))
}

/// 断られた命令（C# の例外）。理由の文は言語の違いで揃わないので比べない。
struct Refused;
impl<E> From<E> for Refused
where
    E: std::error::Error,
{
    fn from(_: E) -> Self {
        Refused
    }
}

#[derive(Default)]
struct Case {
    name: String,
    doc: Option<Document>,
    occlusion: Option<Vec<u8>>,
    pad_width: i64,
    pad_height: i64,
    triangles: Vec<[DVec2; 3]>,
    pad_image: Vec<u8>,
    keep: Vec<bool>,
    /// 掛けた塗り広げの、段 1 の候補（残さない画素で、8 近傍に残す画素がある）の最大。既定の並列の下限を超える事例があるかの確かめ用。
    widest_first_ring: usize,
    lines: Vec<String>,
    bin: Vec<u8>,
}

/// 塗り広げの段 1 の候補の数（残さない画素で、画像の中の 8 近傍に残す画素が 1 つでもあるもの）。
fn first_ring(keep: &[bool], width: usize, height: usize) -> usize {
    (0..keep.len())
        .filter(|&i| {
            let (x, y) = ((i % width) as i64, (i / width) as i64);
            !keep[i]
                && (-1..=1).any(|dy| {
                    (-1..=1).any(|dx| {
                        let (nx, ny) = (x + dx, y + dy);
                        (dx != 0 || dy != 0)
                            && (0..width as i64).contains(&nx)
                            && (0..height as i64).contains(&ny)
                            && keep[ny as usize * width + nx as usize]
                    })
                })
        })
        .count()
}

impl Case {
    fn out(&mut self, label: &str, bytes: &[u8]) {
        self.lines.push(format!("out {label} {}", bytes.len()));
        self.bin.extend_from_slice(bytes);
    }
}

fn layer_at(doc: &Document, s: &str) -> LayerId {
    match s.strip_prefix('@') {
        Some(name) => doc
            .layers()
            .iter()
            .find(|l| l.name() == name)
            .unwrap_or_else(|| panic!("層の名前: {s}"))
            .id(),
        None => doc.layers()[int(s) as usize].id(),
    }
}

fn adjust(s: &str) -> Result<AdjustmentSettings, Refused> {
    let (kind, rest) = s.split_once(':').unwrap_or((s, ""));
    let n: Vec<f64> = rest.split(',').filter(|x| !x.is_empty()).map(dbl).collect();
    Ok(match kind {
        "invert" => AdjustmentSettings::invert(),
        "levels" => AdjustmentSettings::levels(n[0], n[1], n[2], n[3], n[4])?,
        "hsl" => AdjustmentSettings::hue_saturation(n[0], n[1], n[2])?,
        _ => panic!("調整: {s}"),
    })
}

fn mode_opacity(
    doc: &mut Document,
    id: LayerId,
    mode_s: &str,
    opacity_s: &str,
    initial: BlendMode,
) -> Result<(), Refused> {
    let m = mode(mode_s);
    let o = dbl(opacity_s);
    if m != initial {
        doc.set_layer_blend_mode(id, m)?;
    }
    if o != 1.0 {
        doc.set_layer_opacity(id, o, false)?;
    }
    Ok(())
}

fn flag(doc: &mut Document, id: LayerId, flag: &str) -> Result<(), Refused> {
    match flag {
        "hidden" => doc.set_layer_visible(id, false)?,
        "clip" => doc.set_layer_clipping(id, true)?,
        _ => panic!("印: {flag}"),
    }
    Ok(())
}

/// 層の中身: empty | random:種（画布の全画素を下の行から）| sparse:種（タイルごとに 無し・一様・画素）| solid:R,G,B,A。
fn fill(doc: &mut Document, id: LayerId, ch: Channel, spec: &str) {
    let (w, h, ts) = (
        doc.width() as usize,
        doc.height() as usize,
        doc.tile_size() as usize,
    );
    let (columns, rows) = (w.div_ceil(ts), h.div_ceil(ts));
    let put =
        |doc: &mut Document, tx: usize, ty: usize, f: &mut dyn FnMut(usize, usize) -> Rgba8| {
            let mut bytes = vec![0u8; ts * ts * 4];
            for y in 0..ts {
                if ty * ts + y >= h {
                    break;
                }
                for x in 0..ts {
                    if tx * ts + x >= w {
                        break;
                    }
                    let p = f(tx * ts + x, ty * ts + y);
                    bytes[(y * ts + x) * 4..][..4].copy_from_slice(&p.to_array());
                }
            }
            doc.import_tile(id, ch, TileCoord::new(tx as u32, ty as u32), &bytes)
                .expect("読み込み");
        };
    if spec == "empty" {
        return;
    }
    if let Some(seed) = spec.strip_prefix("random:") {
        let mut rng = SplitMix(seed.parse().unwrap());
        let canvas: Vec<Rgba8> = (0..w * h).map(|_| rng.rgba()).collect();
        for ty in 0..rows {
            for tx in 0..columns {
                put(doc, tx, ty, &mut |x, y| canvas[y * w + x]);
            }
        }
    } else if let Some(seed) = spec.strip_prefix("sparse:") {
        let mut rng = SplitMix(seed.parse().unwrap());
        for ty in 0..rows {
            for tx in 0..columns {
                let kind = rng.next() % 4;
                if kind == 0 {
                    continue;
                }
                let uniform = if kind == 1 {
                    rng.rgba()
                } else {
                    Rgba8::TRANSPARENT
                };
                put(doc, tx, ty, &mut |_, _| {
                    if kind == 1 {
                        uniform
                    } else {
                        rng.rgba()
                    }
                });
            }
        }
    } else if let Some(c) = spec.strip_prefix("solid:") {
        let c = color(c);
        for ty in 0..rows {
            for tx in 0..columns {
                put(doc, tx, ty, &mut |_, _| c);
            }
        }
    } else {
        panic!("中身: {spec}");
    }
}

fn command(c: &mut Case, t: &[&str]) -> Result<(), Refused> {
    if t[0] == "canvas" {
        c.doc = Some(Document::with_tile_size(
            int(t[1]) as u32,
            int(t[2]) as u32,
            int(t[3]) as u32,
        )?);
        return Ok(());
    }
    match t[0] {
        "layer" | "paint" | "chenable" | "group" | "fill" | "adjust" | "normal" => {
            let mut doc = c.doc.take().expect("canvas の前");
            let r = document_command(&mut doc, t);
            c.doc = Some(doc);
            r
        }
        "occlusion" => {
            let doc = c.doc.as_ref().expect("canvas の前");
            let n = doc.width() as usize * doc.height() as usize;
            c.occlusion = if t[1] == "none" {
                None
            } else if let Some(seed) = t[1].strip_prefix("random:") {
                let mut rng = SplitMix(seed.parse().unwrap());
                Some((0..n).map(|_| (rng.next() >> 8) as u8).collect())
            } else if let Some(v) = t[1].strip_prefix("flat:") {
                Some(vec![v.parse().unwrap(); n])
            } else {
                panic!("occlusion: {}", t[1])
            };
            Ok(())
        }
        "export" => {
            let template = ExportTemplate::built_in_by_id(t[1]).expect("テンプレート");
            let doc = c.doc.take().expect("canvas の前");
            let has = c.occlusion.is_some();
            let mut result = Ok(());
            for image in &template.images {
                c.lines.push(format!(
                    "should {} {} {}",
                    template.id,
                    image.suffix(),
                    u8::from(should_write(&doc, image, has))
                ));
                match build(&doc, image, c.occlusion.as_deref(), u64::MAX) {
                    Ok(bytes) => c.out(&format!("{} {}", template.id, image.suffix()), &bytes),
                    Err(e) => {
                        result = Err(Refused::from(e));
                        break;
                    }
                }
            }
            c.doc = Some(doc);
            result
        }
        "size" => {
            c.pad_width = int(t[1]);
            c.pad_height = int(t[2]);
            Ok(())
        }
        "tri" => {
            let v: Vec<f64> = t[1..7].iter().map(|s| dbl(s)).collect();
            c.triangles.push([
                DVec2::new(v[0], v[1]),
                DVec2::new(v[2], v[3]),
                DVec2::new(v[4], v[5]),
            ]);
            Ok(())
        }
        "cleartris" => {
            c.triangles.clear();
            Ok(())
        }
        "tris" => {
            let mut rng = SplitMix(t[1].parse().unwrap());
            let count = int(t[2]);
            let size = dbl(t[3]);
            let (w, h) = (c.pad_width as f64, c.pad_height as f64);
            for _ in 0..count {
                let x0 = (rng.u01() * 1.2 - 0.1) * w;
                let y0 = (rng.u01() * 1.2 - 0.1) * h;
                let x1 = x0 + (rng.u01() * 2.0 - 1.0) * size;
                let y1 = y0 + (rng.u01() * 2.0 - 1.0) * size;
                let x2 = x0 + (rng.u01() * 2.0 - 1.0) * size;
                let y2 = y0 + (rng.u01() * 2.0 - 1.0) * size;
                let mut tri = [DVec2::new(x0, y0), DVec2::new(x1, y1), DVec2::new(x2, y2)];
                if rng.next().is_multiple_of(4) {
                    for p in &mut tri {
                        // C# の Math.Round（偶数丸め）
                        *p = DVec2::new(p.x.round_ties_even(), p.y.round_ties_even());
                    }
                }
                c.triangles.push(tri);
            }
            Ok(())
        }
        "coverage" => {
            if c.pad_width <= 0 || c.pad_height <= 0 {
                return Err(Refused);
            }
            let covered = coverage(
                c.pad_width as u32,
                c.pad_height as u32,
                c.triangles.iter().copied(),
            )?;
            let bytes: Vec<u8> = covered.iter().map(|&b| u8::from(b)).collect();
            c.keep = covered;
            c.out("coverage", &bytes);
            Ok(())
        }
        "keep" => {
            let n = (c.pad_width * c.pad_height) as usize;
            c.keep = if t[1] == "all" {
                vec![true; n]
            } else if let Some(rest) = t[1].strip_prefix("random:") {
                let (seed, percent) = rest.split_once(':').expect("random:種:パーセント");
                let mut rng = SplitMix(seed.parse().unwrap());
                let percent = dbl(percent);
                (0..n).map(|_| rng.u01() * 100.0 < percent).collect()
            } else if t[1] == "none" {
                vec![false; n]
            } else {
                panic!("keep: {}", t[1])
            };
            Ok(())
        }
        "image" => {
            let n = (c.pad_width * c.pad_height) as usize;
            let mut image = vec![0u8; n * 4];
            if let Some(rgba) = t[1].strip_prefix("solid:") {
                let p = color(rgba);
                for px in image.chunks_exact_mut(4) {
                    px.copy_from_slice(&p.to_array());
                }
            } else {
                let (kind, seed) = t[1].split_once(':').expect("image: 種類:種");
                let mut rng = SplitMix(seed.parse().unwrap());
                for px in image.chunks_exact_mut(4) {
                    let p = rng.rgba();
                    let a = match kind {
                        "opaque" => 255,
                        "clear" => 0,
                        "random" => p.a,
                        _ => panic!("image: {}", t[1]),
                    };
                    px.copy_from_slice(&[p.r, p.g, p.b, a]);
                }
            }
            c.pad_image = image;
            Ok(())
        }
        "dilate" => {
            let mut budget = u64::MAX;
            for extra in &t[2..] {
                budget = extra
                    .strip_prefix("budget=")
                    .unwrap_or_else(|| panic!("dilate: {extra}"))
                    .parse()
                    .unwrap();
            }
            let reach = Reach::from_setting(int(t[1]) as i32)?;
            if !matches!(reach, Reach::Texels(0))
                && c.keep.len() == (c.pad_width * c.pad_height) as usize
            {
                c.widest_first_ring = c.widest_first_ring.max(first_ring(
                    &c.keep,
                    c.pad_width as usize,
                    c.pad_height as usize,
                ));
            }
            let out = dilate(
                &c.pad_image,
                c.pad_width as u32,
                c.pad_height as u32,
                &c.keep,
                reach,
                budget,
            )?;
            let label = if budget == u64::MAX {
                format!("dilate {}", t[1])
            } else {
                format!("dilate {} budget={budget}", t[1])
            };
            c.out(&label, &out);
            Ok(())
        }
        other => panic!("命令: {other}"),
    }
}

fn document_command(doc: &mut Document, t: &[&str]) -> Result<(), Refused> {
    match t[0] {
        "layer" => {
            let id = doc.add_layer(t[1])?;
            mode_opacity(doc, id, t[2], t[3], BlendMode::Normal)?;
            for f in &t[5..] {
                flag(doc, id, f)?;
            }
            fill(doc, id, Channel::Color, t[4]);
            doc.clear_history()?;
        }
        "paint" => {
            let id = layer_at(doc, t[1]);
            let ch = channel(t[2]);
            doc.set_channel_enabled(id, ch, true)?;
            fill(doc, id, ch, t[3]);
            doc.clear_history()?;
        }
        "chenable" => {
            let id = layer_at(doc, t[1]);
            doc.set_channel_enabled(id, channel(t[2]), t[3] == "1")?;
            doc.clear_history()?;
        }
        "group" => {
            let id = doc.add_group(t[1], None)?;
            mode_opacity(doc, id, t[2], t[3], BlendMode::PassThrough)?;
            for f in &t[4..] {
                flag(doc, id, f)?;
            }
            doc.clear_history()?;
        }
        "fill" => {
            let mut values = Vec::new();
            let mut flags = Vec::new();
            for tok in &t[4..] {
                match tok.split_once('=') {
                    Some((ch, v)) => values.push((channel(ch), color(v))),
                    None => flags.push(*tok),
                }
            }
            let id = doc.add_fill_layer(t[1], &values, None)?;
            mode_opacity(doc, id, t[2], t[3], BlendMode::Normal)?;
            for f in flags {
                flag(doc, id, f)?;
            }
            doc.clear_history()?;
        }
        "adjust" => {
            let mut settings = None;
            let mut only: Option<Vec<Channel>> = None;
            let mut flags = Vec::new();
            for tok in &t[4..] {
                if *tok == "hidden" || *tok == "clip" {
                    flags.push(*tok);
                } else if let Some(list) = tok.strip_prefix("only=") {
                    only = Some(list.split(',').map(channel).collect());
                } else {
                    settings = Some(adjust(tok)?);
                }
            }
            let id =
                doc.add_adjustment_layer(t[1], settings.expect("調整"), only.as_deref(), None)?;
            mode_opacity(doc, id, t[2], t[3], BlendMode::Normal)?;
            for f in flags {
                flag(doc, id, f)?;
            }
            doc.clear_history()?;
        }
        "normal" => {
            let edges = match t[3] {
                "wrap" => HeightEdgeMode::Wrap,
                "clamp" => HeightEdgeMode::Clamp,
                e => panic!("端: {e}"),
            };
            let dir = match t[4] {
                "dx" => NormalYDirection::DirectX,
                "gl" => NormalYDirection::OpenGL,
                d => panic!("向き: {d}"),
            };
            doc.set_normal_settings(
                NormalSettings::new(t[1] == "1", dbl(t[2]), edges, dir)?,
                false,
            )?;
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// 台本を走らせた事例（名前、出来事の行、出力のバイト列）。
fn run_script() -> Vec<Case> {
    let text = std::fs::read_to_string(golden_dir().join("cases.txt")).expect("cases.txt");
    let mut cases: Vec<Case> = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        let line = raw.split('#').next().unwrap();
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        if t[0] == "case" {
            cases.push(Case {
                name: t[1].to_string(),
                ..Case::default()
            });
            continue;
        }
        let c = cases
            .last_mut()
            .unwrap_or_else(|| panic!("{}: case の前に命令がある", n + 1));
        if command(c, &t).is_err() {
            c.lines.push(format!("refused {}", t[0]));
        }
    }
    cases
}

/// index.txt: 事例ごとの出来事の行。
fn read_index() -> (Vec<String>, BTreeMap<String, Vec<String>>) {
    let text = std::fs::read_to_string(golden_dir().join("index.txt"))
        .expect("index.txt が無い（tools/csharp-golden/run.sh export で作る）");
    let mut order = Vec::new();
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix("case ") {
            order.push(name.to_string());
            map.insert(name.to_string(), Vec::new());
            current = Some(name.to_string());
        } else {
            map.get_mut(current.as_ref().expect("case の前の行"))
                .unwrap()
                .push(line.to_string());
        }
    }
    (order, map)
}

/// 出力のバイト列の最初の違いが、どの出力（ラベル）の何バイト目か。
fn locate(lines: &[String], at: usize) -> String {
    let mut offset = 0usize;
    for line in lines {
        if let Some(rest) = line.strip_prefix("out ") {
            let (label, len) = rest.rsplit_once(' ').unwrap();
            let len: usize = len.parse().unwrap();
            if at < offset + len {
                return format!(
                    "{label} の {} バイト目（画素 {}）",
                    at - offset,
                    (at - offset) / 4
                );
            }
            offset += len;
        }
    }
    format!("{at} バイト目")
}

#[test]
fn matches_the_csharp_core_byte_for_byte() {
    let (order, expected) = read_index();
    let cases = run_script();
    let names: Vec<&str> = cases.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        order.iter().map(String::as_str).collect::<Vec<_>>(),
        "台本の事例と index.txt の事例が違う（run.sh export を回し直す）"
    );
    let (mut outputs, mut shoulds, mut refusals, mut bytes) = (0usize, 0usize, 0usize, 0usize);
    let mut failures = Vec::new();
    for c in &cases {
        let want = &expected[&c.name];
        if &c.lines != want {
            let at = c
                .lines
                .iter()
                .zip(want)
                .position(|(a, b)| a != b)
                .unwrap_or(c.lines.len().min(want.len()));
            failures.push(format!(
                "{}: 出来事の行が違う（{} 行目 Rust「{}」C#「{}」）",
                c.name,
                at + 1,
                c.lines.get(at).map_or("-", String::as_str),
                want.get(at).map_or("-", String::as_str)
            ));
            continue;
        }
        let golden = std::fs::read(golden_dir().join(format!("{}.bin", c.name)))
            .unwrap_or_else(|_| panic!("{}.bin が無い", c.name));
        if c.bin != golden {
            let at = c
                .bin
                .iter()
                .zip(&golden)
                .position(|(a, b)| a != b)
                .unwrap_or(c.bin.len().min(golden.len()));
            failures.push(format!(
                "{}: バイトが違う（最初の違いは {}。Rust {} バイト・C# {} バイト）",
                c.name,
                locate(&c.lines, at),
                c.bin.len(),
                golden.len()
            ));
            continue;
        }
        outputs += c.lines.iter().filter(|l| l.starts_with("out ")).count();
        shoulds += c.lines.iter().filter(|l| l.starts_with("should ")).count();
        refusals += c.lines.iter().filter(|l| l.starts_with("refused ")).count();
        bytes += c.bin.len();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    println!(
        "C# と一致: {} 事例、出力 {outputs} 件（{bytes} バイト）、書くかどうか {shoulds} 件、断った {refusals} 件",
        cases.len()
    );
    for c in cases.iter().filter(|c| c.widest_first_ring > 0) {
        println!("  段 1 の候補の最大 {:>6}: {}", c.widest_first_ring, c.name);
    }
    // 台本が空に近くなって、照合が素通りしていないことの確かめ
    assert!(outputs >= 300 && shoulds >= 100 && refusals >= 4);
    // C# との全バイト一致が、既定の並列の枝（段の候補が parallel_min 以上）を通る事例を含むこと
    let widest = cases.iter().map(|c| c.widest_first_ring).max().unwrap_or(0);
    assert!(
        widest >= Tuning::DEFAULT.parallel_min,
        "段 1 の候補が最大 {widest} 個で、並列の下限 {} に届かない（並列の枝が C# と照らされていない）",
        Tuning::DEFAULT.parallel_min
    );
}
