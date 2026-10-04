//! Unity 版の C# の Core の出力（tests/golden、tools/csharp-golden/run.sh で作る）とのバイト一致。
//! 台本（golden/cases.txt）を C# と同じ規則で走らせ、出来事の行（ダブの数・Undo の結果・断られた命令）と出力の画素を比べる。
//! 乱数・台本の読み方は tools/csharp-golden/Golden.cs と揃えてある（片方を変えたら両方を変える）。
#![allow(clippy::chunks_exact_to_as_chunks)]

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use std::sync::Arc;

use yolu_core::blend::{blend, clip_onto, fade};
use yolu_core::brush::random::NetRandom;
use yolu_core::brush::{
    curve, hsv_to_rgb, linear_to_srgb, luminance, pen_tilt, rgb_to_hsv, BUILTIN_TIPS,
};
use yolu_core::glam::DVec2;
use yolu_core::{
    builtin_tip, BlendMode, Brush, BrushEffect, BrushPixel, BrushSample, BrushSettings,
    BrushStencil, BrushTip, Channel, ColorDynamics, Document, DualBrush, DualBrushMode,
    ImageColorSpace, PaperTexture, Rect, Rgba8, StencilImage, StencilMapping, StencilMode,
    StencilPoint, StencilTiling, Stroke, TileCoord, TipSelection,
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
    /// stencil 命令で作った今のステンシル（stroke の stencil=1 で付ける）。
    stencil: Option<Arc<BrushStencil>>,
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
fn channel(s: &str) -> Channel {
    *Channel::ALL
        .iter()
        .find(|c| format!("{c:?}") == s)
        .unwrap_or_else(|| panic!("チャンネル: {s}"))
}
fn tip(id: &str) -> Arc<BrushTip> {
    builtin_tip(id).unwrap_or_else(|| panic!("筆先: {id}"))
}
fn flag(v: &str) -> bool {
    v == "1"
}

/// 台本のキーから組み立てる途中のブラシ（紙の質感・効果は値が揃ってから作る。C# は別々の項目）。
struct BrushBuild {
    brush: Brush,
    texture: Option<Arc<BrushTip>>,
    depth: f64,
    scale: f64,
    effect: Option<&'static str>,
    blur: u32,
    smudge: f64,
    clone: DVec2,
}

impl BrushBuild {
    fn new(base: BrushSettings) -> Self {
        BrushBuild {
            brush: Brush::from(base),
            texture: None,
            depth: 0.0,
            scale: 1.0,
            effect: None,
            blur: 3,
            smudge: 0.5,
            clone: DVec2::ZERO,
        }
    }
    fn finish(mut self) -> Brush {
        self.brush.texture = self.texture.map(|image| PaperTexture {
            image,
            depth: self.depth,
            scale: self.scale,
            mode: yolu_core::TextureMode::Multiply,
        });
        self.brush.effect = match self.effect {
            None | Some("Paint") => BrushEffect::Paint,
            Some("Blur") => BrushEffect::Blur { radius: self.blur },
            Some("Smudge") => BrushEffect::Smudge {
                strength: self.smudge,
            },
            Some("Clone") => BrushEffect::Clone { offset: self.clone },
            Some(e) => panic!("effect: {e}"),
        };
        self.brush
    }
}

fn dual(b: &mut BrushBuild) -> &mut DualBrush {
    b.brush.dual.as_mut().expect("dual= の前に d のキー")
}

/// M1 より後のブラシのキー（tools/csharp-golden/Golden.cs の BrushKey と同じ並び・同じ読み方）。
fn brush_key(c: &mut CaseRun, b: &mut BrushBuild, k: &str, v: &str) -> bool {
    let br = &mut b.brush;
    match k {
        "seed" => br.seed = int(v) as i32,
        "tip" => br.tip.image = Some(tip(v)),
        "tips" => br.tip.images = v.split(',').map(tip).collect(),
        "tipsel" => {
            br.tip.selection = match v {
                "seq" => TipSelection::Sequential,
                "random" => TipSelection::Random,
                _ => panic!("tipsel: {v}"),
            }
        }
        "angle" => br.tip.angle = num(c, v),
        "roundness" => br.tip.roundness = num(c, v),
        "follow" => br.tip.follow_direction = flag(v),
        "sizejit" => br.jitter.size = num(c, v),
        "anglejit" => br.jitter.angle = num(c, v),
        "roundjit" => br.jitter.roundness = num(c, v),
        "opjit" => br.jitter.opacity = num(c, v),
        "flowjit" => br.jitter.flow = num(c, v),
        "scatter" => br.jitter.scatter = num(c, v),
        "count" => br.jitter.count = int(v) as u32,
        "texture" => b.texture = Some(tip(v)),
        "tdepth" => b.depth = num(c, v),
        "tscale" => b.scale = num(c, v),
        "stabilizer" => br.assist.stabilizer = num(c, v),
        "taperin" => br.assist.taper_in = num(c, v),
        "taperout" => br.assist.taper_out = num(c, v),
        "curve" => br.assist.curve = flag(v),
        "secondary" => br.color.secondary = color(v),
        "fbj" => br.color.foreground_background = num(c, v),
        "huejit" => br.color.hue = num(c, v),
        "satjit" => br.color.saturation = num(c, v),
        "brightjit" => br.color.brightness = num(c, v),
        "purity" => br.color.purity = num(c, v),
        "pertip" => br.color.per_tip = flag(v),
        "fadesize" => br.controls.fade_size = int(v) as u32,
        "fadeop" => br.controls.fade_opacity = int(v) as u32,
        "fadeflow" => br.controls.fade_flow = int(v) as u32,
        "tiltsize" => br.controls.tilt_size = flag(v),
        "tiltop" => br.controls.tilt_opacity = flag(v),
        "tiltflow" => br.controls.tilt_flow = flag(v),
        "tiltangle" => br.controls.tilt_angle = flag(v),
        "effect" => {
            b.effect = Some(match v {
                "Paint" => "Paint",
                "Blur" => "Blur",
                "Smudge" => "Smudge",
                "Clone" => "Clone",
                _ => panic!("effect: {v}"),
            })
        }
        "blur" => b.blur = int(v) as u32,
        "smudge" => b.smudge = num(c, v),
        "clone" => {
            let (x, y) = v.split_once(',').expect("clone=X,Y");
            b.clone = DVec2::new(num(c, x), num(c, y));
        }
        "stencil" => {
            if flag(v) {
                br.stencil = Some(c.stencil.clone().expect("stencil= の前に stencil 命令"));
            }
        }
        "dual" => {
            br.dual = Some(DualBrush {
                tip: if v == "round" { None } else { Some(tip(v)) },
                ..DualBrush::default()
            })
        }
        "dradius" => dual(b).radius = num(c, v),
        "dhard" => dual(b).hardness = num(c, v),
        "dspacing" => dual(b).spacing = num(c, v),
        "dangle" => dual(b).angle = num(c, v),
        "dround" => dual(b).roundness = num(c, v),
        "dscatter" => dual(b).scatter = num(c, v),
        "dcount" => dual(b).count = int(v) as u32,
        "dmode" => {
            dual(b).mode = *DualBrushMode::ALL
                .iter()
                .find(|m| format!("{m:?}") == v)
                .unwrap_or_else(|| panic!("dmode: {v}"))
        }
        _ => return false,
    }
    true
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

/// ステンシルの画像（Golden.cs の StencilPixels と同じ）: grey:種:W:H、color:種:W:H、varied:W:H、half:W:H。
fn stencil_pixels(spec: &str) -> (Vec<u8>, usize, usize) {
    let q: Vec<&str> = spec.split(':').collect();
    let seeded = q[0] == "grey" || q[0] == "color";
    let (w, h) = if seeded {
        (int(q[2]) as usize, int(q[3]) as usize)
    } else {
        (int(q[1]) as usize, int(q[2]) as usize)
    };
    let mut rng = if seeded {
        Some(SplitMix(q[1].parse().unwrap()))
    } else {
        None
    };
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let p = match q[0] {
                "grey" => {
                    let r = rng.as_mut().unwrap();
                    let v = r.channel();
                    Rgba8::new(v, v, v, r.alpha())
                }
                "color" => rng.as_mut().unwrap().rgba(),
                "varied" => Rgba8::new(
                    (x * 13 % 256) as u8,
                    (y * 29 % 256) as u8,
                    ((x * y + 7) % 256) as u8,
                    255,
                ),
                "half" => {
                    if x < w / 2 {
                        Rgba8::new(255, 255, 255, 255)
                    } else {
                        Rgba8::new(0, 0, 0, 255)
                    }
                }
                _ => panic!("ステンシルの画像: {spec}"),
            };
            rgba[(y * w + x) * 4..][..4].copy_from_slice(&p.to_array());
        }
    }
    (rgba, w, h)
}

/// 台本のストローク（C# は null の札に命令すると例外 = 断られた）。
fn stroke_of(c: &mut CaseRun) -> Result<&mut Stroke, yolu_core::CoreError> {
    c.stroke
        .as_mut()
        .ok_or(yolu_core::CoreError::NoActiveStroke)
}

fn channel_bytes(doc: &Document, index: usize, ch: Channel) -> Vec<u8> {
    match doc.layers()[index].surface(ch) {
        Some(s) => s.to_canvas_bytes(),
        None => vec![0; doc.width() as usize * doc.height() as usize * 4],
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
            let mut b = BrushBuild::new(BrushSettings::default());
            let mut ch = Channel::Color;
            for kv in &t[2..] {
                let (k, v) = kv.split_once('=').expect("キー=値");
                let s = &mut b.brush.base;
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
                    "channel" => ch = channel(v),
                    _ => {
                        if !brush_key(c, &mut b, k, v) {
                            panic!("stroke のキー: {k}");
                        }
                    }
                }
            }
            assert!(c.stroke.is_none(), "ストロークが重なっている");
            let brush = b.finish();
            c.stroke = Some(doc.begin_brush_stroke_in(layer_at(doc, t[1]), ch, &brush)?);
        }
        "tpoint" => {
            let (x, y, p) = (num(c, t[1]), num(c, t[2]), num(c, t[3]));
            let (time, tx, ty) = (num(c, t[4]), num(c, t[5]), num(c, t[6]));
            let sample = BrushSample::new(x, y, p, time, DVec2::new(tx, ty)).expect("tpoint");
            stroke_of(c)?.add_sample(doc, sample)?;
        }
        "twalk" => {
            let mut rng = SplitMix(int(t[1]) as u64);
            let count = int(t[2]);
            let (mut x, mut y, step) = (num(c, t[3]), num(c, t[4]), num(c, t[5]));
            for _ in 0..count {
                let p = 0.1 + 0.9 * rng.u01();
                let tx = (rng.u01() * 2.0 - 1.0) * 1.5;
                let ty = (rng.u01() * 2.0 - 1.0) * 1.5;
                let sample = BrushSample::new(x, y, p, 0.0, DVec2::new(tx, ty)).expect("twalk");
                stroke_of(c)?.add_sample(doc, sample)?;
                x += (rng.u01() * 2.0 - 1.0) * step;
                y += (rng.u01() * 2.0 - 1.0) * step;
            }
        }
        "dab" => {
            let (cx, cy, p) = (num(c, t[1]), num(c, t[2]), num(c, t[3]));
            let mut pixels = Vec::new();
            for q in &t[4..] {
                let q: Vec<&str> = q.split(':').collect();
                pixels.push(BrushPixel {
                    x: int(q[0]),
                    y: int(q[1]),
                    coverage: num(c, q[2]),
                });
            }
            stroke_of(c)?.apply_dab(doc, &pixels, DVec2::new(cx, cy), p)?;
        }
        "dabdisc" => {
            let (cx, cy, r, p) = (num(c, t[1]), num(c, t[2]), num(c, t[3]), num(c, t[4]));
            let mut pixels = Vec::new();
            for y in (cy - r).floor() as i64..=(cy + r).ceil() as i64 {
                for x in (cx - r).floor() as i64..=(cx + r).ceil() as i64 {
                    let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                    let d = (dx * dx + dy * dy).sqrt() / r;
                    if d < 1.0 {
                        pixels.push(BrushPixel {
                            x,
                            y,
                            coverage: 1.0 - d,
                        });
                    }
                }
            }
            stroke_of(c)?.apply_dab(doc, &pixels, DVec2::new(cx, cy), p)?;
        }
        "resetdir" => stroke_of(c)?.reset_effect_direction(doc)?,
        "stencil" => {
            let mode = match t[1] {
                "Auto" => StencilMode::Auto,
                "Mask" => StencilMode::Mask,
                "Color" => StencilMode::Color,
                m => panic!("ステンシルのモード: {m}"),
            };
            let tiling = match t[2] {
                "None" => StencilTiling::None,
                "Horizontal" => StencilTiling::Horizontal,
                "Vertical" => StencilTiling::Vertical,
                "Both" => StencilTiling::Both,
                m => panic!("ステンシルの繰り返し: {m}"),
            };
            let invert = flag(t[3]);
            let mapping = if t[4] == "none" {
                None
            } else {
                let q: Vec<&str> = t[4].split(',').collect();
                let v: Vec<f64> = q.iter().map(|x| num(c, x)).collect();
                Some(StencilMapping::new(v[0], v[1], v[2], v[3], v[4], v[5]).expect("写し"))
            };
            let (pixels, w, h) = stencil_pixels(t[5]);
            let space = match t[6] {
                "Unspecified" => ImageColorSpace::Unspecified,
                "Srgb" => ImageColorSpace::Srgb,
                "Linear" => ImageColorSpace::Linear,
                m => panic!("色空間: {m}"),
            };
            let channels: Vec<Channel> = if t[7] == "-" {
                Vec::new()
            } else {
                t[7].split(',').map(channel).collect()
            };
            let image =
                StencilImage::new(w, h, pixels, space, StencilImage::DEFAULT_MIP_BUDGET_BYTES)
                    .expect("ステンシルの画像");
            c.stencil = Some(Arc::new(BrushStencil::new(
                Arc::new(image),
                mode,
                tiling,
                invert,
                mapping,
                &channels,
            )));
        }
        "pixelat" => {
            let (x, y) = (int(t[1]), int(t[2]));
            let (cov, p, sx, sy, foot) = (
                num(c, t[3]),
                num(c, t[4]),
                num(c, t[5]),
                num(c, t[6]),
                num(c, t[7]),
            );
            let at = StencilPoint::new(sx, sy, foot).expect("点");
            stroke_of(c)?.apply_pixel_at(doc, x, y, cov, p, at)?;
        }
        "dabdiscat" => {
            let (cx, cy, r, p) = (num(c, t[1]), num(c, t[2]), num(c, t[3]), num(c, t[4]));
            let q: Vec<f64> = t[5].split(',').map(|x| num(c, x)).collect();
            let m = StencilMapping::new(q[0], q[1], q[2], q[3], q[4], q[5]).expect("写し");
            let (mut pixels, mut points) = (Vec::new(), Vec::new());
            for y in (cy - r).floor() as i64..=(cy + r).ceil() as i64 {
                for x in (cx - r).floor() as i64..=(cx + r).ceil() as i64 {
                    let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                    let d = (dx * dx + dy * dy).sqrt() / r;
                    if d >= 1.0 {
                        continue;
                    }
                    pixels.push(BrushPixel {
                        x,
                        y,
                        coverage: 1.0 - d,
                    });
                    let (ix, iy) = m.map(x, y);
                    points.push(StencilPoint::new(ix, iy, m.footprint()).expect("点"));
                }
            }
            stroke_of(c)?.apply_dab_at(doc, &pixels, DVec2::new(cx, cy), p, &points)?;
        }
        "budget" => doc.set_stroke_budget_bytes(int(t[1]) as u64)?,
        "enable" => {
            // C# は SetChannelEnabled（面ができる）と履歴の消去。Rust はストロークを始めるときに面を作るので、履歴だけ消す
            let _ = channel(t[2]);
            doc.clear_history()?;
        }
        "point" => {
            let (x, y, p) = (num(c, t[1]), num(c, t[2]), num(c, t[3]));
            stroke_of(c)?.add_point(doc, x, y, p, DVec2::ZERO)?;
        }
        "walk" => {
            let mut rng = SplitMix(int(t[1]) as u64);
            let count = int(t[2]);
            let (mut x, mut y, step) = (num(c, t[3]), num(c, t[4]), num(c, t[5]));
            for _ in 0..count {
                let p = 0.1 + 0.9 * rng.u01();
                stroke_of(c)?.add_point(doc, x, y, p, DVec2::ZERO)?;
                x += (rng.u01() * 2.0 - 1.0) * step;
                y += (rng.u01() * 2.0 - 1.0) * step;
            }
        }
        "pixel" => {
            let (x, y) = (int(t[1]), int(t[2]));
            let (cov, p) = (num(c, t[3]), num(c, t[4]));
            stroke_of(c)?.apply_pixel(doc, x, y, cov, p)?;
        }
        "commit" => {
            c.parallel_dabs += doc.active_stroke_stats().map_or(0, |st| st.parallel_dabs);
            let s = c
                .stroke
                .take()
                .ok_or(yolu_core::CoreError::NoActiveStroke)?;
            let r = doc.end_stroke(s)?;
            c.events.push(format!(
                "commit stamps={} samples={} changed={}",
                r.stamps, r.samples, r.changed as u8
            ));
        }
        "cancel" => {
            doc.cancel_stroke(
                c.stroke
                    .take()
                    .ok_or(yolu_core::CoreError::NoActiveStroke)?,
            );
            c.events.push("cancel".into());
        }
        "stats" => {
            let st = doc
                .active_stroke_stats()
                .ok_or(yolu_core::CoreError::NoActiveStroke)?;
            c.events.push(format!(
                "stats stamps={} samples={} tiles={} rollback={}",
                st.stamps, st.samples, st.tiles, st.rollback_bytes
            ));
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
                "channel" => (
                    channel_bytes(doc, int(t[2]) as usize, channel(t[3])),
                    format!("channel {} {} {}x{}", t[2], t[3], doc.width(), doc.height()),
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
                    stencil: None,
                },
            ));
            continue;
        }
        let c = &mut cases.last_mut().expect("case の前の命令").1;
        let r = command(c, &t);
        if let (Err(e), Ok(_)) = (&r, std::env::var("GOLDEN_TRACE")) {
            eprintln!("{}: {} → {e:?}", cases.last().unwrap().0, line.trim());
        }
        let c = &mut cases.last_mut().expect("case の前の命令").1;
        if r.is_err() {
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
    let by_case: Vec<String> = cases
        .iter()
        .filter(|(_, c)| c.parallel_dabs > 0)
        .map(|(n, c)| format!("{n} {}", c.parallel_dabs))
        .collect();
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
        "C# の Core とバイト一致: {} 事例、{outputs} 出力、{bytes} バイト（ワーカーで描いたダブ {parallel}: {}）",
        cases.len(),
        by_case.join(", ")
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

/// ブラシの式の掃引（Golden.cs の BrushSweeps と同じ乱数・同じ順）: System.Random の列、組み込みの筆先の画素、筆先の双線形、
/// 色の変化、デュアルの合わせ方、ペンの傾き、曲線の点、HSV。
#[test]
fn brush_formulas_match_the_csharp_core_on_sweeps() {
    let (_, index) = read_index();
    let (_, events) = index.get("brush_sweeps").expect("brush_sweeps が無い");
    let mut mine = Vec::new();
    {
        let mut f = Fnv::new();
        for seed in [
            0,
            1,
            -1,
            7,
            i32::MIN,
            i32::MAX,
            0x2545F491,
            0x5DEECE6,
            123456789,
        ] {
            let mut r = NetRandom::new(seed);
            for _ in 0..4096 {
                f.double(r.next_double());
            }
            for i in 1..300 {
                f.double(r.next_below(i) as f64);
            }
        }
        mine.push(format!("sweep random {}", f.hex()));
    }
    for id in BUILTIN_TIPS {
        let t = builtin_tip(id).unwrap();
        let mut f = Fnv::new();
        for v in [t.width(), t.height()] {
            f.byte(v as u8);
            f.byte((v >> 8) as u8);
        }
        for &b in t.alpha() {
            f.byte(b);
        }
        mine.push(format!("sweep tip_{id} {}", f.hex()));
    }
    {
        let mut rng = SplitMix(3000);
        let mut f = Fnv::new();
        let (tip, grain) = (
            builtin_tip("charcoal").unwrap(),
            builtin_tip("grain").unwrap(),
        );
        for _ in 0..65536 {
            let u = rng.u01() * 1.2 - 0.1;
            let v = rng.u01() * 1.2 - 0.1;
            f.double(tip.sample(u, v));
            let x = (rng.u01() - 0.5) * 1000.0;
            let y = (rng.u01() - 0.5) * 1000.0;
            f.double(grain.sample_tiled(x, y));
        }
        mine.push(format!("sweep tip_sample {}", f.hex()));
    }
    {
        let mut rng = SplitMix(3001);
        let mut f = Fnv::new();
        let maybe = |rng: &mut SplitMix| {
            if rng.next() & 1 == 0 {
                0.0
            } else {
                rng.u01()
            }
        };
        for _ in 0..4096 {
            let color = rng.rgba();
            let secondary = rng.rgba();
            let foreground_background = maybe(&mut rng);
            let hue = maybe(&mut rng);
            let saturation = maybe(&mut rng);
            let brightness = maybe(&mut rng);
            let purity = if rng.next().is_multiple_of(3) {
                0.0
            } else {
                rng.u01() * 2.0 - 1.0
            };
            let cd = ColorDynamics {
                secondary,
                foreground_background,
                hue,
                saturation,
                brightness,
                purity,
                per_tip: true,
            };
            let mut r = NetRandom::new((rng.next() & 0x7fffffff) as i32);
            for _ in 0..8 {
                f.rgba(cd.next(color, &mut r));
            }
        }
        mine.push(format!("sweep color_dynamics {}", f.hex()));
    }
    {
        let mut rng = SplitMix(3002);
        let mut f = Fnv::new();
        let value = |rng: &mut SplitMix| {
            if rng.next().is_multiple_of(5) {
                (rng.next() % 3) as f64 * 0.5
            } else {
                rng.u01()
            }
        };
        for mode in DualBrushMode::ALL {
            for _ in 0..16384 {
                let a = value(&mut rng);
                let b = value(&mut rng);
                f.double(mode.combine(a, b));
            }
        }
        mine.push(format!("sweep dual_combine {}", f.hex()));
    }
    {
        let mut rng = SplitMix(3003);
        let mut f = Fnv::new();
        let value = |rng: &mut SplitMix| {
            if rng.next().is_multiple_of(7) {
                0.0
            } else {
                (rng.u01() * 2.0 - 1.0) * pen_tilt::MAX_ANGLE
            }
        };
        for _ in 0..65536 {
            let tx = value(&mut rng);
            let ty = value(&mut rng);
            f.double(pen_tilt::amount(tx, ty));
            f.double(pen_tilt::azimuth(tx, ty));
        }
        mine.push(format!("sweep pen_tilt {}", f.hex()));
    }
    {
        let mut rng = SplitMix(3004);
        let mut f = Fnv::new();
        for _ in 0..65536 {
            let mut q = [0.0; 8];
            for v in q.iter_mut() {
                *v = (rng.u01() - 0.5) * 200.0;
            }
            if rng.next().is_multiple_of(9) {
                q[0] = q[2];
                q[1] = q[3];
            }
            let t = rng.u01();
            let (x, y) = curve::point(q[0], q[1], q[2], q[3], q[4], q[5], q[6], q[7], t);
            f.double(x);
            f.double(y);
        }
        mine.push(format!("sweep curve {}", f.hex()));
    }
    {
        let mut rng = SplitMix(3005);
        let mut f = Fnv::new();
        for _ in 0..65536 {
            let (r, g, b) = (rng.u01(), rng.u01(), rng.u01());
            let (h, s, v) = rgb_to_hsv(r, g, b);
            f.double(h);
            f.double(s);
            f.double(v);
            let (h, s, v) = (rng.u01() * 3.0 - 1.0, rng.u01(), rng.u01());
            let (r, g, b) = hsv_to_rgb(h, s, v);
            f.double(r);
            f.double(g);
            f.double(b);
        }
        mine.push(format!("sweep hsv {}", f.hex()));
    }
    {
        let mut f = Fnv::new();
        for i in 0..=255u8 {
            f.byte(linear_to_srgb(i));
        }
        let mut rng = SplitMix(3006);
        for _ in 0..65536 {
            let (r, g, b) = (rng.channel(), rng.channel(), rng.channel());
            f.byte(luminance(r, g, b));
        }
        mine.push(format!("sweep srgb_luminance {}", f.hex()));
    }
    for spec in [
        "grey:41:37:23",
        "color:42:64:48",
        "color:43:1:7",
        "half:5:5",
    ] {
        let (pixels, w, h) = stencil_pixels(spec);
        let image = StencilImage::new(
            w,
            h,
            pixels,
            ImageColorSpace::Srgb,
            StencilImage::DEFAULT_MIP_BUDGET_BYTES,
        )
        .unwrap();
        let mut rng = SplitMix(3007);
        let mut f = Fnv::new();
        let footprints = [0.0, 0.5, 1.0, 1.7, 3.0, 9.0, 100.0, 1e6];
        let tilings = [
            StencilTiling::None,
            StencilTiling::Horizontal,
            StencilTiling::Vertical,
            StencilTiling::Both,
        ];
        for _ in 0..16384 {
            let x = (rng.u01() * 2.0 - 0.5) * w as f64;
            let y = (rng.u01() * 2.0 - 0.5) * h as f64;
            let foot = footprints[(rng.next() % 8) as usize];
            let tiling = tilings[(rng.next() % 4) as usize];
            let t = image.read(x, y, foot, tiling);
            f.byte(t.inside as u8);
            f.double(t.alpha);
            f.double(t.luma_alpha);
            f.rgba(t.color);
        }
        f.double(image.mip_bytes() as f64);
        mine.push(format!(
            "sweep stencil_{} {}",
            spec.replace(':', "_"),
            f.hex()
        ));
    }
    let wrong: Vec<_> = events
        .iter()
        .zip(&mine)
        .filter(|(a, b)| a != b)
        .map(|(a, b)| format!("正解 {a} / Rust {b}"))
        .collect();
    assert_eq!(events.len(), mine.len(), "掃引の数: 正解 {events:?}");
    assert!(
        wrong.is_empty(),
        "ブラシの式が C# と違う:\n{}",
        wrong.join("\n")
    );
}
