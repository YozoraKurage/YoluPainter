//! Unity 版の C# の Core の出力（tests/golden、tools/csharp-golden/run.sh で作る）とのバイト一致。
//! 台本（golden/cases.txt）を C# と同じ規則で走らせ、出来事の行（ダブの数・Undo の結果・断られた命令）と出力の画素を比べる。
//! 乱数・台本の読み方は tools/csharp-golden/Golden.cs と揃えてある（片方を変えたら両方を変える）。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use yolu_core::blend::{blend, clip_onto, fade};
use yolu_core::glam::DVec2;
use yolu_core::{BlendMode, BrushSettings, Document, Rect, Rgba8, Stroke, TileCoord};

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
    fn opacity(&mut self) -> f64 {
        match self.next() % 8 {
            0 => 1.0,
            1 => 0.0,
            2 => 0.5,
            3 => 1.0 / 255.0,
            4 => f64::from_bits(1), // C# の double.Epsilon
            5 => 0.99999999,
            _ => self.u01(),
        }
    }
}

struct Fnv(u64);
impl Fnv {
    fn new() -> Self {
        Fnv(14695981039346656037)
    }
    fn byte(&mut self, b: u8) {
        self.0 ^= b as u64;
        self.0 = self.0.wrapping_mul(1099511628211);
    }
    fn rgba(&mut self, c: Rgba8) {
        for b in c.to_array() {
            self.byte(b);
        }
    }
    fn double(&mut self, d: f64) {
        for b in d.to_bits().to_le_bytes() {
            self.byte(b);
        }
    }
    fn hex(&self) -> String {
        format!("{:016x}", self.0)
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// 事例の名前 → (params の指紋, 出来事の行)。
type Index = HashMap<String, (String, Vec<String>)>;

/// index.txt: 事例ごとの params の指紋と出来事の行。
fn read_index() -> (Vec<String>, Index) {
    let text = std::fs::read_to_string(golden_dir().join("index.txt"))
        .expect("index.txt が無い（tools/csharp-golden/run.sh で作る）");
    let mut order = Vec::new();
    let mut map: HashMap<String, (String, Vec<String>)> = HashMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("case ") {
            let (name, params) = rest.split_once(" params=").expect("case の行");
            order.push(name.to_string());
            map.insert(name.to_string(), (params.to_string(), Vec::new()));
            current = Some(name.to_string());
        } else {
            map.get_mut(current.as_ref().expect("case の前の行"))
                .unwrap()
                .1
                .push(line.to_string());
        }
    }
    (order, map)
}

struct CaseRun {
    params: Fnv,
    events: Vec<String>,
    outputs: Vec<Vec<u8>>,
    doc: Option<Document>,
    stroke: Option<Stroke>,
    /// ワーカーで描いた大きなダブの数（並列の経路も C# と照らせているかの確かめ）。
    parallel_dabs: u64,
}

fn num(c: &mut CaseRun, s: &str) -> f64 {
    let v: f64 = s.parse().unwrap_or_else(|_| panic!("数: {s}"));
    c.params.double(v);
    v
}
fn int(s: &str) -> i64 {
    s.parse().unwrap_or_else(|_| panic!("整数: {s}"))
}
fn color(s: &str) -> Rgba8 {
    let p: Vec<u8> = s.split(',').map(|v| v.parse().expect("色")).collect();
    Rgba8::new(p[0], p[1], p[2], p[3])
}
fn mode(s: &str) -> BlendMode {
    BlendMode::from_name(s).unwrap_or_else(|| panic!("モード: {s}"))
}

fn fill(doc: &mut Document, id: yolu_core::LayerId, spec: &str) {
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
            doc.import_tile(
                id,
                yolu_core::Channel::Color,
                TileCoord::new(tx as u32, ty as u32),
                &bytes,
            )
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

fn layer_bytes(doc: &Document, index: usize) -> Vec<u8> {
    doc.layers()[index]
        .surface(yolu_core::Channel::Color)
        .expect("Color の面")
        .to_canvas_bytes()
}

/// 命令 1 つ。Err は「断られた」（C# の例外）。
fn command(c: &mut CaseRun, t: &[&str]) -> Result<(), yolu_core::CoreError> {
    if t[0] == "canvas" {
        c.doc = Some(Document::with_tile_size(
            int(t[1]) as u32,
            int(t[2]) as u32,
            int(t[3]) as u32,
        )?);
        return Ok(());
    }
    let mut doc = c.doc.take().expect("canvas の前");
    let r = command_on(c, &mut doc, t);
    c.doc = Some(doc);
    r
}

fn command_on(c: &mut CaseRun, doc: &mut Document, t: &[&str]) -> Result<(), yolu_core::CoreError> {
    let layer_at = |doc: &Document, s: &str| doc.layers()[int(s) as usize].id();
    match t[0] {
        "layer" => {
            let id = doc.add_layer(t[1])?;
            let m = mode(t[2]);
            let opacity = num(c, t[3]);
            if m != BlendMode::Normal {
                doc.set_layer_blend_mode(id, m)?;
            }
            if opacity != 1.0 {
                doc.set_layer_opacity(id, opacity, false)?;
            }
            let mut spec = None;
            for tok in &t[4..] {
                match *tok {
                    "hidden" => doc.set_layer_visible(id, false)?,
                    "clip" => doc.set_layer_clipping(id, true)?,
                    other => spec = Some(other),
                }
            }
            fill(doc, id, spec.expect("layer の中身"));
            doc.clear_history()?;
        }
        "add" => {
            doc.add_layer(t[1])?;
        }
        "stroke" => {
            let mut s = BrushSettings::default();
            for kv in &t[2..] {
                let (k, v) = kv.split_once('=').expect("キー=値");
                match k {
                    "radius" => s.radius = num(c, v),
                    "hardness" => s.hardness = num(c, v),
                    "spacing" => s.spacing = num(c, v),
                    "opacity" => s.opacity = num(c, v),
                    "flow" => s.flow = num(c, v),
                    "color" => s.color = color(v),
                    "psize" => s.pressure_size = v == "1",
                    "popacity" => s.pressure_opacity = v == "1",
                    "pflow" => s.pressure_flow = v == "1",
                    "erase" => s.erase = v == "1",
                    _ => panic!("stroke のキー: {k}"),
                }
            }
            assert!(c.stroke.is_none(), "ストロークが重なっている");
            c.stroke = Some(doc.begin_stroke(layer_at(doc, t[1]), &s)?);
        }
        "point" => {
            let (x, y, p) = (num(c, t[1]), num(c, t[2]), num(c, t[3]));
            c.stroke
                .as_mut()
                .expect("stroke の後")
                .add_point(doc, x, y, p, DVec2::ZERO)?;
        }
        "walk" => {
            let mut rng = SplitMix(int(t[1]) as u64);
            let count = int(t[2]);
            let (mut x, mut y, step) = (num(c, t[3]), num(c, t[4]), num(c, t[5]));
            for _ in 0..count {
                let p = 0.1 + 0.9 * rng.u01();
                c.stroke
                    .as_mut()
                    .expect("stroke の後")
                    .add_point(doc, x, y, p, DVec2::ZERO)?;
                x += (rng.u01() * 2.0 - 1.0) * step;
                y += (rng.u01() * 2.0 - 1.0) * step;
            }
        }
        "pixel" => {
            let (x, y) = (int(t[1]), int(t[2]));
            let (cov, p) = (num(c, t[3]), num(c, t[4]));
            c.stroke
                .as_mut()
                .expect("stroke の後")
                .apply_pixel(doc, x, y, cov, p)?;
        }
        "commit" => {
            c.parallel_dabs += doc.active_stroke_stats().map_or(0, |st| st.parallel_dabs);
            let s = c.stroke.take().expect("stroke の後");
            let r = doc.end_stroke(s)?;
            c.events.push(format!(
                "commit stamps={} samples={} changed={}",
                r.stamps, r.samples, r.changed as u8
            ));
        }
        "cancel" => {
            doc.cancel_stroke(c.stroke.take().expect("stroke の後"));
            c.events.push("cancel".into());
        }
        "undo" => {
            let r = doc.undo()?;
            c.events.push(format!("undo {}", r as u8));
        }
        "redo" => {
            let r = doc.redo()?;
            c.events.push(format!("redo {}", r as u8));
        }
        "visible" => doc.set_layer_visible(layer_at(doc, t[1]), t[2] == "1")?,
        "opacity" => {
            let v = num(c, t[2]);
            doc.set_layer_opacity(layer_at(doc, t[1]), v, false)?
        }
        "mode" => doc.set_layer_blend_mode(layer_at(doc, t[1]), mode(t[2]))?,
        "clip" => doc.set_layer_clipping(layer_at(doc, t[1]), t[2] == "1")?,
        "move" => doc.move_layer(layer_at(doc, t[1]), int(t[2]) as usize)?,
        "remove" => doc.remove_layer(layer_at(doc, t[1]))?,
        "out" => {
            let (bytes, what) = match t[1] {
                "composite" => (
                    doc.composite(doc.bounds())?,
                    format!("composite {}x{}", doc.width(), doc.height()),
                ),
                "layer" => (
                    layer_bytes(doc, int(t[2]) as usize),
                    format!("layer {} {}x{}", t[2], doc.width(), doc.height()),
                ),
                "region" => {
                    let (x, y, w, h) = (
                        int(t[2]) as u32,
                        int(t[3]) as u32,
                        int(t[4]) as u32,
                        int(t[5]) as u32,
                    );
                    (
                        doc.composite(Rect::new(x, y, w, h))?,
                        format!("region {x} {y} {w}x{h}"),
                    )
                }
                other => panic!("out: {other}"),
            };
            c.events.push(format!("out {} {}", c.outputs.len(), what));
            c.outputs.push(bytes);
        }
        other => panic!("命令: {other}"),
    }
    Ok(())
}

fn run_script() -> Vec<(String, CaseRun)> {
    let text = std::fs::read_to_string(golden_dir().join("cases.txt")).expect("cases.txt");
    let mut cases: Vec<(String, CaseRun)> = Vec::new();
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap();
        let t: Vec<&str> = line.split_whitespace().collect();
        if t.is_empty() {
            continue;
        }
        if t[0] == "case" {
            cases.push((
                t[1].to_string(),
                CaseRun {
                    params: Fnv::new(),
                    events: Vec::new(),
                    outputs: Vec::new(),
                    doc: None,
                    stroke: None,
                    parallel_dabs: 0,
                },
            ));
            continue;
        }
        let c = &mut cases.last_mut().expect("case の前の命令").1;
        if command(c, &t).is_err() {
            c.events.push(format!("error {}", t[0]));
            let active = c.doc.as_ref().is_some_and(|d| d.has_active_stroke());
            if !active {
                c.stroke = None; // 途中の失敗でストロークは取り消された
            }
        }
    }
    for (name, c) in &cases {
        assert!(
            c.stroke.is_none(),
            "{name}: 確定も取消もしていないストロークがある"
        );
    }
    cases
}

/// 最初に違う画素の説明。
fn first_difference(expected: &[u8], actual: &[u8], width: usize) -> String {
    if expected.len() != actual.len() {
        return format!(
            "長さが違う: 正解 {} / Rust {}",
            expected.len(),
            actual.len()
        );
    }
    let count = expected
        .chunks_exact(4)
        .zip(actual.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    let i = expected
        .chunks_exact(4)
        .zip(actual.chunks_exact(4))
        .position(|(a, b)| a != b)
        .unwrap();
    format!(
        "{count} 画素が違う。最初は ({}, {}): 正解 {:?} / Rust {:?}",
        i % width,
        i / width,
        &expected[i * 4..i * 4 + 4],
        &actual[i * 4..i * 4 + 4]
    )
}

#[test]
fn cases_match_the_csharp_core_byte_for_byte() {
    let (_, index) = read_index();
    let cases = run_script();
    let mut failures = String::new();
    let mut outputs = 0;
    let mut bytes = 0;
    for (name, c) in &cases {
        let Some((params, events)) = index.get(name) else {
            let _ = writeln!(failures, "{name}: index.txt に無い（run.sh で作り直す）");
            continue;
        };
        if *params != c.params.hex() {
            let _ = writeln!(
                failures,
                "{name}: 台本の数の読み方が C# と違う（params {} / {}）",
                params,
                c.params.hex()
            );
        }
        if *events != c.events {
            let _ = writeln!(
                failures,
                "{name}: 出来事が違う\n  正解: {events:?}\n  Rust: {:?}",
                c.events
            );
            continue;
        }
        for (n, out) in c.outputs.iter().enumerate() {
            let path = golden_dir().join(format!("{name}.{n}.rgba"));
            let expected =
                std::fs::read(&path).unwrap_or_else(|_| panic!("{} が無い", path.display()));
            let line = &c
                .events
                .iter()
                .filter(|e| e.starts_with("out "))
                .nth(n)
                .unwrap();
            let width: usize = line
                .rsplit(' ')
                .next()
                .unwrap()
                .split('x')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            if expected != *out {
                let _ = writeln!(
                    failures,
                    "{name} の出力 {n}（{line}）: {}",
                    first_difference(&expected, out, width)
                );
            }
            outputs += 1;
            bytes += out.len();
        }
    }
    assert!(failures.is_empty(), "C# の Core と違う:\n{failures}");
    let parallel: u64 = cases.iter().map(|(_, c)| c.parallel_dabs).sum();
    assert!(
        rayon::current_num_threads() == 1 || parallel > 0,
        "並列の経路を通ったダブが無い"
    );
    assert!(
        cases.len() >= 60 && outputs >= 80,
        "事例 {} / 出力 {outputs}（少なすぎる）",
        cases.len()
    );
    eprintln!(
        "C# の Core とバイト一致: {} 事例、{outputs} 出力、{bytes} バイト（ワーカーで描いたダブ {parallel}）",
        cases.len()
    );
}

#[test]
fn pixel_formulas_match_the_csharp_core_on_random_sweeps() {
    let (_, index) = read_index();
    let (_, events) = index.get("sweeps").expect("sweeps が無い");
    let mut mine = Vec::new();
    for m in BlendMode::LAYER_MODES {
        let mut rng = SplitMix(1000 + m as u64);
        let (mut b, mut cl) = (Fnv::new(), Fnv::new());
        for _ in 0..65536 {
            let d = rng.rgba();
            let s = rng.rgba();
            let op = rng.opacity();
            b.rgba(blend(d, s, op, m));
            cl.rgba(clip_onto(d, s, op, m));
        }
        mine.push(format!("sweep blend_{} {}", m.name(), b.hex()));
        mine.push(format!("sweep clip_{} {}", m.name(), cl.hex()));
    }
    let mut rng = SplitMix(2000);
    let mut f = Fnv::new();
    for _ in 0..65536 {
        let d = rng.rgba();
        let s = rng.rgba();
        let op = rng.opacity();
        f.rgba(fade(d, s, op));
    }
    mine.push(format!("sweep fade {}", f.hex()));
    let wrong: Vec<_> = events
        .iter()
        .zip(&mine)
        .filter(|(a, b)| a != b)
        .map(|(a, b)| format!("正解 {a} / Rust {b}"))
        .collect();
    assert_eq!(events.len(), mine.len());
    assert!(
        wrong.is_empty(),
        "画素の式が C# と違う:\n{}",
        wrong.join("\n")
    );
}

/// 行の核（合成の速い経路）が画素ごとの参照の式（composite_pixel）と同じバイトか（C# の CompositorExactnessTests に当たる）。
#[test]
fn region_composite_equals_the_per_pixel_reference() {
    for (name, c) in run_script() {
        let Some(doc) = c.doc else { continue };
        let all = doc.composite(doc.bounds()).unwrap();
        let w = doc.width() as usize;
        for y in 0..doc.height() {
            for x in 0..doc.width() {
                let p = doc
                    .composite_pixel(yolu_core::Channel::Color, x, y)
                    .unwrap();
                let i = (y as usize * w + x as usize) * 4;
                assert_eq!(&all[i..i + 4], &p.to_array(), "{name} ({x},{y})");
            }
        }
    }
}
