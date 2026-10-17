//! 取り込んだ CLIP STUDIO PAINT のブラシ（`.sut`）で描いた線と、CLIP STUDIO PAINT が書き出した画像との差を数で出す（式を合わせる外の基準）。
//!
//! データはリポジトリに入れない。置き場は環境変数で受ける（無ければ何もせずに通る）:
//! - `YOLU_CSP_DIFF_DIR`: 書き出しのフォルダ（直下に `task4/spray`・`task5/{stab,dual,mix}`・`task6/dual2`。各フォルダに `manifest.csv`）
//! - `YOLU_CSP_COLOR_DIR`: 色の変化のフォルダ（`manifest.csv` と `.sut`・線の画像）
//! - `YOLU_CSP_DIFF_OUT`: 差の画像（差の大きさを白黒に。白が差なし）を書くフォルダ（無ければ書かない）
//!
//! `cargo test -p yolu-io --test brushes csp_line_compare:: -- --ignored --nocapture` で節ごとの表が出る。下地混色の節（`task5/mix`）は、色の帯を敷いた面の
//! 上へ描き（帯は基準の画像の最上段の行を下へ伸ばしたもの）、表の行の下に、線の中心の行の RGB の差（帯ごと）と、帯の中心の色を並べる。ブラシの名前・ファイル名は
//! コードに写さない（行は manifest から読み、表にだけ出す）。コードに置くのは、比べる画像を描いたときの型（キャンバスの大きさ、
//! 8 か所のクリックの並び、直線の端）だけで、サイズ・色・間隔・線の座標・筆圧は manifest の note と `.sut` から読む。
//!
//! 比べ方: 両方を白の上に重ね、「暗さ」（255 − 輝度）の差と RGB の差を出す。位置のずれは重心（暗さで重みをつけた中心）の差を出すだけで、
//! ずらして合わせない。座標は左上が原点の値を、core の左下が原点へ `高さ − y` で写す（クリック位置はそのまま使い、半画素の補正は入れない。
//! ずれは重心の差に出る）。
//! 乱数を引く物（色の変化・散布の粒子の位置）は CLIP STUDIO PAINT と同じ列にならないので、そうした行は画素の差でなく、インクの量・色・
//! 塊の数・中心からの距離で見る。ストロークごとの乱数の種は、ストロークの番号から決める（アプリが毎回変えるのと同じ）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushSample, Document, Rect, Rgba8, RowOrder};
use yolu_io::brushes::import;

/// 比べる画像のキャンバス（幅 1000・高さ 300・白）。
const W: u32 = 1000;
const H: u32 = 300;
/// 差が超えた画素を数える段（暗さの 0〜255）。
const STEPS: [u8; 3] = [8, 32, 64];
/// 点の絵の 8 か所のクリック（x 70 から 120 おき・y 150）。
const CLICK_XS: [f64; 8] = [70.0, 190.0, 310.0, 430.0, 550.0, 670.0, 790.0, 910.0];
const CLICK_Y: f64 = 150.0;
/// 色の線の端（x 50 から x 950 の Shift の直線・y 150）。
const LINE_FROM: (f64, f64) = (50.0, 150.0);
const LINE_TO: (f64, f64) = (950.0, 150.0);
/// 折れ線を、入力の列へ直す刻み（画素）。入力の細かさは manifest に無いので、ここで仮に決めた値。
const PATH_STEP: f64 = 4.0;

// ---------------- manifest ----------------

#[derive(Clone, Debug, Default)]
struct Row {
    file: String,
    group: String,
    item_en: String,
    to: String,
    screenshot: String,
    note: String,
}

/// RFC 4180 の CSV（引用符の中の改行・"" を扱う。先頭の BOM を除く）。
fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = Vec::new();
    let (mut row, mut field, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (quoted, c) {
            (true, '"') if chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            (true, '"') => quoted = false,
            (true, c) => field.push(c),
            (false, '"') => quoted = true,
            (false, ',') => row.push(std::mem::take(&mut field)),
            (false, '\r') => {}
            (false, '\n') => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
            }
            (false, c) => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn read_manifest(dir: &Path) -> Option<Vec<Row>> {
    let text = std::fs::read_to_string(dir.join("manifest.csv")).ok()?;
    let mut rows = parse_csv(&text).into_iter();
    let header = rows.next()?;
    let col: HashMap<&str, usize> = header
        .iter()
        .enumerate()
        .map(|(i, h)| (h.as_str(), i))
        .collect();
    let get = |r: &Vec<String>, name: &str| {
        col.get(name)
            .and_then(|i| r.get(*i))
            .cloned()
            .unwrap_or_default()
    };
    Some(
        rows.filter(|r| r.iter().any(|f| !f.is_empty()))
            .map(|r| Row {
                file: get(&r, "file"),
                group: get(&r, "group"),
                item_en: get(&r, "item_en"),
                to: get(&r, "to"),
                screenshot: get(&r, "screenshot"),
                note: get(&r, "note"),
            })
            .collect(),
    )
}

// ---------------- 文字からの読み取り ----------------

/// 文字列の中の数（符号・小数つき）を、出てくる順に。`段階`（段階のバーの段数）の数は値ではないので除く。
fn numbers(s: &str) -> Vec<f64> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let signed = (c == '-' || c == '+') && chars.get(i + 1).is_some_and(|d| d.is_ascii_digit());
        if c.is_ascii_digit() || signed {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            let text: String = chars[start..i].iter().collect();
            let bar = chars.get(i) == Some(&'段');
            if let (Ok(v), false) = (text.trim_end_matches('.').parse::<f64>(), bar) {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

fn first_number(s: &str) -> Option<f64> {
    numbers(s).first().copied()
}

/// `key` の直後（空白か `=`・`:` をはさんでよい）にある数。key が複数あれば、直後に数があるものの最初。
fn number_after(s: &str, key: &str) -> Option<f64> {
    s.match_indices(key).find_map(|(at, _)| {
        let rest = s[at + key.len()..].trim_start_matches([' ', '=', ':', '\u{3000}']);
        let head: String = rest.chars().take(24).collect();
        let starts = head
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '+');
        first_number(&head).filter(|_| starts)
    })
}

/// `#RRGGBB`（key の直後。key が複数あれば、直後に色があるものの最初）。
fn color_after(s: &str, key: &str) -> Option<Rgba8> {
    s.match_indices(key).find_map(|(at, _)| {
        let hex = s[at + key.len()..]
            .trim_start()
            .strip_prefix('#')?
            .get(..6)?;
        let v = u32::from_str_radix(hex, 16).ok()?;
        Some(Rgba8::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
    })
}

/// `(x,y)` が矢印か横線だけでつながった列のうち、最初の 2 点以上のもの（折れ線・Shift の直線）。
fn parse_path(note: &str) -> Option<Vec<(f64, f64)>> {
    let mut groups: Vec<Vec<(f64, f64)>> = vec![Vec::new()];
    let chars: Vec<char> = note.chars().collect();
    let mut i = 0;
    let mut between = String::new();
    while i < chars.len() {
        if chars[i] == '(' {
            let close = chars[i..].iter().position(|c| *c == ')').map(|p| i + p);
            let inner: Option<(f64, f64)> = close.and_then(|c| {
                let t: String = chars[i + 1..c].iter().collect();
                let (a, b) = t.split_once(',')?;
                Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
            });
            if let (Some(p), Some(c)) = (inner, close) {
                let linked = !between.trim().is_empty()
                    && between
                        .trim()
                        .chars()
                        .all(|c| matches!(c, '-' | '>' | '→' | '–' | '—'));
                if !linked && !groups.last().is_some_and(Vec::is_empty) {
                    groups.push(Vec::new());
                }
                groups.last_mut().unwrap().push(p);
                between.clear();
                i = c + 1;
                continue;
            }
        }
        between.push(chars[i]);
        if between.chars().count() > 12 {
            between.clear();
            if !groups.last().is_some_and(Vec::is_empty) {
                groups.push(Vec::new());
            }
        }
        i += 1;
    }
    groups.into_iter().find(|g| g.len() >= 2)
}

/// 点の位置と筆圧の並び（「x 70・190 = 0.2」の形。y は点の絵の高さ）。
fn parse_pressure_clicks(note: &str) -> Vec<((f64, f64), f64)> {
    let mut out = Vec::new();
    for (at, _) in note.match_indices("x ") {
        let segment: String = note[at + 2..]
            .chars()
            .take_while(|c| !matches!(c, '、' | '。' | ';'))
            .collect();
        let Some((left, right)) = segment.split_once('=') else {
            continue;
        };
        if left.is_empty()
            || left
                .chars()
                .any(|c| !(c.is_ascii_digit() || c == '・' || c == ' '))
        {
            continue;
        }
        let Some(pressure) = first_number(right) else {
            continue;
        };
        out.extend(
            left.split('・')
                .filter_map(|x| x.trim().parse::<f64>().ok())
                .map(|x| ((x, CLICK_Y), pressure)),
        );
    }
    out
}

/// 筆圧一定の横の線の並び（「x 50→950」と、線ごとの「y=60(筆圧0.2)」）。
fn parse_pressure_lines(note: &str) -> Vec<Shape> {
    let span = note.match_indices("x ").find_map(|(at, _)| {
        let head: String = note[at + 2..].chars().take(12).collect();
        let n = numbers(&head);
        (head.contains('→') && n.len() >= 2).then(|| (n[0], n[1]))
    });
    let Some((x0, x1)) = span else {
        return Vec::new();
    };
    note.match_indices("y=")
        .filter_map(|(at, _)| {
            let rest = &note[at + 2..];
            let y = first_number(rest)?;
            let key = rest.find("(筆圧")? + "(筆圧".len();
            let pressure = first_number(&rest[key..])?;
            Some(Shape::Line {
                from: (x0, y),
                to: (x1, y),
                pressure: (pressure, pressure),
            })
        })
        .collect()
}

/// `(x,y)` の形の点がいくつ書いてあるか。
fn count_points(note: &str) -> usize {
    note.match_indices('(')
        .filter(|(at, _)| {
            let inner: String = note[at + 1..].chars().take_while(|c| *c != ')').collect();
            inner.split_once(',').is_some_and(|(a, b)| {
                a.trim().parse::<f64>().is_ok() && b.trim().parse::<f64>().is_ok()
            })
        })
        .count()
}

// ---------------- 描き方 ----------------

#[derive(Clone, Debug)]
enum Shape {
    /// 点（1 回ずつのクリック）。
    Clicks(Vec<(f64, f64)>),
    /// Shift の直線（始めと終わりの点。筆圧は始めと終わりで、間は直線で補う）。
    Line {
        from: (f64, f64),
        to: (f64, f64),
        pressure: (f64, f64),
    },
    /// 折れ線をなぞった動き（一定の速さ。始めから終わりまで `seconds`）。
    Path {
        points: Vec<(f64, f64)>,
        seconds: f64,
    },
    /// 点を、筆圧ごとに 1 回ずつ置く（点の位置・筆圧）。点は 1 画素だけ右へ動かして置く（動きの無い 1 点だけの入力は何も置かない物があるため）。
    Nudged(Vec<((f64, f64), f64)>),
    /// 続けて描く（順に別のストローク）。
    Many(Vec<Shape>),
}

impl Shape {
    /// 点の絵の、クリックの中心（左上原点）。
    fn centres(&self) -> Vec<(f64, f64)> {
        match self {
            Shape::Clicks(points) => points.clone(),
            Shape::Nudged(points) => points.iter().map(|(p, _)| *p).collect(),
            _ => Vec::new(),
        }
    }
}

/// 1 行を描くのに、`.sut` の設定の上に重ねる値（その線を描いたときの状態。`.sut` に入っていない物）。
#[derive(Clone, Debug, Default)]
struct Setup {
    /// ブラシの直径（画素）。
    size: Option<f64>,
    main: Option<Rgba8>,
    sub: Option<Rgba8>,
    /// 固定の間隔（直径に対する %）。
    interval_pct: Option<f64>,
    spray_size: Option<f64>,
    spray_link: bool,
    spray_density: Option<f64>,
    spray_bias: Option<f64>,
    /// 先端の色の変化の値の上書き（`.sut` に対応する書き出しが無い値）: (項目, 値)。
    tip_value: Option<(String, f64)>,
}

struct Case {
    label: String,
    sut: PathBuf,
    reference: PathBuf,
    shape: Shape,
    setup: Setup,
    /// 描き方の控え（表の最後に出す）。
    how: String,
    /// 下地（幅 1000・高さ 300 の RGB、上の行から）。あれば、最初のレイヤーをこの絵で塗ってから描く（無ければ透明で、白の上に重ねる）。
    background: Option<Vec<u8>>,
}

fn apply_setup(brush: &mut Brush, s: &Setup) {
    if let Some(d) = s.size {
        brush.base.radius = d / 2.0;
    }
    brush.base.color = s.main.unwrap_or(Rgba8::new(0, 0, 0, 255));
    if let Some(c) = s.sub {
        brush.color.secondary = c;
    }
    if let Some(p) = s.interval_pct {
        brush.base.spacing = (p / 100.0).clamp(0.01, 4.0);
    }
    if let Some(spray) = brush.spray.as_mut() {
        if let Some(v) = s.spray_size {
            spray.size = v;
        }
        if s.spray_link {
            spray.link_size = true;
        }
        if let Some(v) = s.spray_density {
            spray.density = v;
        }
        if let Some(v) = s.spray_bias {
            spray.bias = v / 100.0;
        }
    }
    if let Some((item, v)) = &s.tip_value {
        let mut tip = brush.color.tip.unwrap_or(yolu_core::TipColorChange {
            hue: 0.0,
            saturation: 0.0,
            value: 0.0,
        });
        match item.as_str() {
            "Hue" => tip.hue = *v,
            "Saturation" => tip.saturation = v / 100.0,
            "Value" => tip.value = v / 100.0,
            "Blend with sub color" => brush.color.sub_mix = v / 100.0,
            _ => {}
        }
        if item != "Blend with sub color" {
            brush.color.tip = Some(tip);
        }
    }
}

/// core の座標（左下原点）。
fn to_core(p: (f64, f64)) -> (f64, f64) {
    (p.0, f64::from(H) - p.1)
}

/// ストロークの番号から決める乱数の種（アプリがストロークごとに変えるのと同じく、毎回違う値。同じ番号なら同じ値）。
fn stroke_seed(n: u32) -> i32 {
    let mut x = n.wrapping_add(1).wrapping_mul(0x9E37_79B9);
    x ^= x >> 15;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x & 0x7fff_ffff) as i32
}

struct Painter<'a> {
    doc: Document,
    layer: yolu_core::LayerId,
    brush: &'a Brush,
    strokes: u32,
}

fn sample(p: (f64, f64), pressure: f64, time: f64) -> Result<BrushSample, String> {
    let (x, y) = to_core(p);
    BrushSample::new(x, y, pressure, time, DVec2::ZERO).map_err(|e| format!("{e:?}"))
}

impl Painter<'_> {
    /// 1 本のストローク（点の列。`end_line` なら最後に直線として届かせる）。
    fn stroke(&mut self, points: &[BrushSample], end_line: bool) -> Result<(), String> {
        let mut brush = self.brush.clone();
        brush.seed = stroke_seed(self.strokes);
        self.strokes += 1;
        let mut stroke = self
            .doc
            .begin_brush_stroke(self.layer, &brush)
            .map_err(|e| format!("{e:?}"))?;
        for p in points {
            stroke
                .add_sample(&mut self.doc, *p)
                .map_err(|e| format!("{e:?}"))?;
        }
        if end_line {
            stroke
                .end_line(&mut self.doc)
                .map_err(|e| format!("{e:?}"))?;
        }
        self.doc.end_stroke(stroke).map_err(|e| format!("{e:?}"))?;
        Ok(())
    }

    /// `step_limit` は、固定の間隔の画素の長さ（0 なら取り込んだ間隔のまま）。
    fn paint(&mut self, shape: &Shape, step_limit: f64) -> Result<(), String> {
        match shape {
            Shape::Clicks(points) => {
                for p in points {
                    self.stroke(&[sample(*p, 1.0, 0.0)?], false)?;
                }
            }
            Shape::Nudged(points) => {
                for (p, pressure) in points {
                    let moved = (p.0 + 1.0, p.1);
                    self.stroke(
                        &[sample(*p, *pressure, 0.0)?, sample(moved, *pressure, 0.15)?],
                        false,
                    )?;
                }
            }
            Shape::Many(shapes) => {
                for s in shapes {
                    self.paint(s, step_limit)?;
                }
            }
            Shape::Line { from, to, pressure } => {
                let diameter = self.brush.base.radius * 2.0;
                let gap = self.brush.base.spacing * diameter;
                let length = (to.0 - from.0).hypot(to.1 - from.1);
                // エンジンの間隔の上限（直径の 4 倍）を超える間隔の線は、始め・間隔ごと・終わりの点に 1 回ずつ置く
                // （書き出しの画像の点の並びが、この置き方と合う）
                let wanted = step_limit;
                if wanted > 0.0 && wanted > gap + 1e-9 && wanted >= 4.0 * diameter - 1e-9 {
                    let mut points = vec![*from];
                    let mut d = wanted;
                    while d < length - 1e-9 {
                        let t = d / length;
                        points.push((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
                        d += wanted;
                    }
                    points.push(*to);
                    for p in points {
                        let t = ((p.0 - from.0).hypot(p.1 - from.1) / length).clamp(0.0, 1.0);
                        let pr = pressure.0 + (pressure.1 - pressure.0) * t;
                        self.stroke(&[sample(p, pr, 0.0)?], false)?;
                    }
                } else {
                    self.stroke(
                        &[
                            sample(*from, pressure.0, 0.0)?,
                            sample(*to, pressure.1, 0.0)?,
                        ],
                        true,
                    )?;
                }
            }
            Shape::Path { points, seconds } => {
                let mut lengths = vec![0.0];
                for w in points.windows(2) {
                    let l = (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
                    lengths.push(lengths.last().unwrap() + l);
                }
                let total = *lengths.last().unwrap();
                let n = (total / PATH_STEP).ceil().max(1.0) as usize;
                let mut samples = Vec::new();
                for k in 0..=n {
                    let d = total * k as f64 / n as f64;
                    let seg = lengths
                        .windows(2)
                        .position(|w| d <= w[1])
                        .unwrap_or(points.len() - 2);
                    let span = (lengths[seg + 1] - lengths[seg]).max(1e-9);
                    let t = ((d - lengths[seg]) / span).clamp(0.0, 1.0);
                    let (a, b) = (points[seg], points[seg + 1]);
                    let p = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    samples.push(sample(p, 1.0, seconds * d / total.max(1e-9))?);
                }
                self.stroke(&samples, false)?;
            }
        }
        Ok(())
    }
}

/// 最初のレイヤーを、RGB（上の行から）の絵で塗る（不透明）。
fn paint_background(
    doc: &mut Document,
    layer: yolu_core::LayerId,
    rgb: &[u8],
) -> Result<(), String> {
    let ts = doc.tile_size();
    for ty in 0..H.div_ceil(ts) {
        for tx in 0..W.div_ceil(ts) {
            let mut bytes = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..ts {
                for x in 0..ts {
                    let (px, py) = (tx * ts + x, ty * ts + y);
                    if px >= W || py >= H {
                        continue;
                    }
                    // core は左下が原点
                    let i = (((H - 1 - py) * W + px) * 3) as usize;
                    let o = ((y * ts + x) * 4) as usize;
                    bytes[o..o + 4].copy_from_slice(&[rgb[i], rgb[i + 1], rgb[i + 2], 255]);
                }
            }
            doc.import_tile(
                layer,
                yolu_core::Channel::Color,
                yolu_core::TileCoord::new(tx, ty),
                &bytes,
            )
            .map_err(|e| format!("{e:?}"))?;
        }
    }
    Ok(())
}

/// ブラシで形のとおりに描き、白の上に重ねた RGB（上の行から）を返す。
fn render(
    brush: &Brush,
    shape: &Shape,
    step_limit: f64,
    background: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    let mut doc = Document::new(W, H).map_err(|e| format!("{e:?}"))?;
    let layer = doc.add_layer("L").map_err(|e| format!("{e:?}"))?;
    if let Some(rgb) = background {
        paint_background(&mut doc, layer, rgb)?;
    }
    let mut painter = Painter {
        doc,
        layer,
        brush,
        strokes: 0,
    };
    painter.paint(shape, step_limit)?;
    let rgba = painter
        .doc
        .composite_into_vec(Rect::new(0, 0, W, H))
        .map_err(|e| format!("{e:?}"))?;
    Ok(over_white(&rgba))
}

trait CompositeTopDown {
    fn composite_into_vec(&self, rect: Rect) -> Result<Vec<u8>, yolu_core::CoreError>;
}
impl CompositeTopDown for Document {
    fn composite_into_vec(&self, rect: Rect) -> Result<Vec<u8>, yolu_core::CoreError> {
        let mut out = vec![0u8; rect.width as usize * rect.height as usize * 4];
        self.composite_into(yolu_core::Channel::Color, rect, &mut out, RowOrder::TopDown)?;
        Ok(out)
    }
}

/// straight RGBA を白の上に重ねた RGB。
fn over_white(rgba: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(rgba.len() / 4 * 3);
    for p in rgba.as_chunks::<4>().0 {
        let a = u32::from(p[3]);
        for c in &p[..3] {
            out.push(((u32::from(*c) * a + 255 * (255 - a) + 127) / 255) as u8);
        }
    }
    out
}

/// 基準の画像（上の行から）を白の上に重ねた RGB で。大きさが違えば Err。
fn read_reference(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("読めない: {e}"))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("大きさが数えられない")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{e}"))?;
    if (info.width, info.height) != (W, H) {
        return Err(format!("大きさが {}x{}", info.width, info.height));
    }
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf[..info.buffer_size()]
            .iter()
            .flat_map(|g| [*g, *g, *g, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf[..info.buffer_size()]
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|p| [p[0], p[0], p[0], p[1]])
            .collect(),
        other => return Err(format!("{other:?} は読まない")),
    };
    Ok(over_white(&rgba))
}

// ---------------- 比べ ----------------

/// 暗さ（255 − 輝度）。
fn darkness(p: &[u8]) -> f64 {
    255.0 - (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
}

#[derive(Debug, Default, PartialEq)]
struct Stats {
    /// 暗さの差の平均（全画素）・最大・段ごとの超えた画素数。
    dark_mean: f64,
    dark_max: f64,
    dark_over: [usize; 3],
    /// 暗さの合計の比（描いた ÷ 基準）。
    ink_ratio: f64,
    /// RGB の差（チャンネルごとの差の最大）の、どちらかにインクがある画素での平均・最大・32 を超えた画素数。
    color_mean: f64,
    color_max: u8,
    color_over32: usize,
    /// 暗さで重みをつけた重心（左上原点の画素）。インクが無ければ None。
    centroid_ours: Option<(f64, f64)>,
    centroid_ref: Option<(f64, f64)>,
    /// インクの色（暗さが最大の 9 割以上の画素の平均）。
    mean_color_ours: Option<[u8; 3]>,
    mean_color_ref: Option<[u8; 3]>,
    /// 比べから外した画素の列の範囲（x の始め）。
    skipped_from_x: Option<u32>,
    /// 暗さが 96 を超える画素のかたまり（4 近傍）の数と、それぞれの重心からクリックの中心までの距離の平均（点の絵だけ）。
    blobs_ours: usize,
    blobs_ref: usize,
    blob_dist_ours: Option<f64>,
    blob_dist_ref: Option<f64>,
}

/// 暗さが 96 を超える画素のかたまり（4 近傍）の重心（左上原点）。`skip_from_x` 以降の列は見ない。
fn blobs(rgb: &[u8], skip_from_x: Option<u32>) -> Vec<(f64, f64)> {
    let w = skip_from_x.unwrap_or(W);
    let ink = |x: u32, y: u32| darkness(&rgb[((y * W + x) * 3) as usize..][..3]) > 96.0;
    let mut seen = vec![false; (W * H) as usize];
    let mut out = Vec::new();
    for y in 0..H {
        for x in 0..w {
            if seen[(y * W + x) as usize] || !ink(x, y) {
                continue;
            }
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
            let mut stack = vec![(x, y)];
            seen[(y * W + x) as usize] = true;
            while let Some((cx, cy)) = stack.pop() {
                sx += f64::from(cx) + 0.5;
                sy += f64::from(cy) + 0.5;
                n += 1.0;
                for (nx, ny) in [
                    (cx.wrapping_sub(1), cy),
                    (cx + 1, cy),
                    (cx, cy.wrapping_sub(1)),
                    (cx, cy + 1),
                ] {
                    if nx < w && ny < H && !seen[(ny * W + nx) as usize] && ink(nx, ny) {
                        seen[(ny * W + nx) as usize] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            out.push((sx / n, sy / n));
        }
    }
    out
}

/// かたまりの重心から、いちばん近いクリックの中心までの距離の平均。
fn mean_distance_to(centres: &[(f64, f64)], blobs: &[(f64, f64)]) -> Option<f64> {
    if blobs.is_empty() || centres.is_empty() {
        return None;
    }
    let sum: f64 = blobs
        .iter()
        .map(|b| {
            centres
                .iter()
                .map(|c| (b.0 - c.0).hypot(b.1 - c.1))
                .fold(f64::INFINITY, f64::min)
        })
        .sum();
    Some(sum / blobs.len() as f64)
}

/// `skip_from_x` 以降の列は比べない（基準の画像に無い物を外す）。
fn compare(
    ours: &[u8],
    reference: &[u8],
    skip_from_x: Option<u32>,
    centres: &[(f64, f64)],
) -> (Stats, Vec<u8>) {
    let mut s = Stats {
        skipped_from_x: skip_from_x,
        ..Stats::default()
    };
    let (mut sum_diff, mut n) = (0.0, 0usize);
    let (mut ink_o, mut ink_r) = (0.0, 0.0);
    let (mut co, mut cr) = ((0.0, 0.0), (0.0, 0.0));
    let (mut mean_o, mut mean_r) = ([0.0; 3], [0.0; 3]);
    let (mut weight_o, mut weight_r) = (0.0, 0.0);
    // インクの色: 暗さが最大の 9 割以上の画素（縁のにじみを除いた芯）の平均
    let core_o = max_darkness(ours) * 0.9;
    let core_r = max_darkness(reference) * 0.9;
    let (mut color_sum, mut color_n) = (0.0, 0usize);
    let mut diff_image = vec![255u8; (W * H) as usize];
    for y in 0..H {
        for x in 0..W {
            if skip_from_x.is_some_and(|from| x >= from) {
                continue;
            }
            let i = (y * W + x) as usize;
            let (po, pr) = (&ours[i * 3..i * 3 + 3], &reference[i * 3..i * 3 + 3]);
            let (dof, drf) = (darkness(po), darkness(pr));
            let diff = (dof - drf).abs();
            sum_diff += diff;
            n += 1;
            s.dark_max = s.dark_max.max(diff);
            for (k, step) in STEPS.iter().enumerate() {
                if diff > f64::from(*step) {
                    s.dark_over[k] += 1;
                }
            }
            diff_image[i] = 255 - diff.round().clamp(0.0, 255.0) as u8;
            ink_o += dof;
            ink_r += drf;
            let (cx, cy) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
            co = (co.0 + dof * cx, co.1 + dof * cy);
            cr = (cr.0 + drf * cx, cr.1 + drf * cy);
            if dof >= core_o && dof > 0.0 {
                for c in 0..3 {
                    mean_o[c] += f64::from(po[c]);
                }
                weight_o += 1.0;
            }
            if drf >= core_r && drf > 0.0 {
                for c in 0..3 {
                    mean_r[c] += f64::from(pr[c]);
                }
                weight_r += 1.0;
            }
            if dof > 8.0 || drf > 8.0 {
                let cd = (0..3).map(|c| po[c].abs_diff(pr[c])).max().unwrap_or(0);
                color_sum += f64::from(cd);
                color_n += 1;
                s.color_max = s.color_max.max(cd);
                if cd > 32 {
                    s.color_over32 += 1;
                }
            }
        }
    }
    s.dark_mean = sum_diff / n.max(1) as f64;
    s.ink_ratio = if ink_r > 0.0 { ink_o / ink_r } else { f64::NAN };
    s.color_mean = color_sum / color_n.max(1) as f64;
    if ink_o > 0.0 {
        s.centroid_ours = Some((co.0 / ink_o, co.1 / ink_o));
    }
    if ink_r > 0.0 {
        s.centroid_ref = Some((cr.0 / ink_r, cr.1 / ink_r));
    }
    let mean = |m: [f64; 3], w: f64| -> Option<[u8; 3]> {
        (w > 0.0).then(|| [0, 1, 2].map(|c| (m[c] / w).round().clamp(0.0, 255.0) as u8))
    };
    let (bo, br) = (blobs(ours, skip_from_x), blobs(reference, skip_from_x));
    s.blobs_ours = bo.len();
    s.blobs_ref = br.len();
    s.blob_dist_ours = mean_distance_to(centres, &bo);
    s.blob_dist_ref = mean_distance_to(centres, &br);
    s.mean_color_ours = mean(mean_o, weight_o);
    s.mean_color_ref = mean(mean_r, weight_r);
    (s, diff_image)
}

fn max_darkness(rgb: &[u8]) -> f64 {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|p| darkness(p))
        .fold(0.0, f64::max)
}

fn write_gray_png(path: &Path, gray: &[u8]) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), W, H);
    enc.set_color(png::ColorType::Grayscale);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .and_then(|mut w| w.write_image_data(gray))
        .map_err(|e| e.to_string())
}

fn header() -> String {
    format!(
        "{:<44} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} | {:>6} {:>6} {:>5} | {:>16} {:>16} {:>13} | {:<13} {:<13} | {:>9} {:>11} | 注記 | 描き方",
        "行",
        "暗さ平均",
        "最大",
        ">8",
        ">32",
        ">64",
        "インク比",
        "色平均",
        "色最大",
        "色>32",
        "重心(描いた)",
        "重心(基準)",
        "差(x,y)",
        "インク色(描)",
        "インク色(基)",
        "塊(描/基)",
        "中心距離(描/基)"
    )
}

fn opt(v: Option<f64>) -> String {
    v.map_or("(.sut)".to_string(), |v| format!("{v}"))
}

fn fmt_point(p: Option<(f64, f64)>) -> String {
    p.map_or("-".to_string(), |(x, y)| format!("{x:.1},{y:.1}"))
}
fn fmt_rgb(c: Option<[u8; 3]>) -> String {
    c.map_or("-".to_string(), |c| format!("{},{},{}", c[0], c[1], c[2]))
}

fn line(label: &str, s: &Stats, notes: usize, how: &str) -> String {
    let delta = match (s.centroid_ours, s.centroid_ref) {
        (Some(a), Some(b)) => format!("{:+.1},{:+.1}", a.0 - b.0, a.1 - b.1),
        _ => "-".to_string(),
    };
    format!(
        "{:<44} {:>8.3} {:>6.0} {:>6} {:>6} {:>6} {:>7.2} | {:>6.2} {:>6} {:>5} | {:>16} {:>16} {:>13} | {:<13} {:<13} | {:>9} {:>11} | 注記{} | {}{}",
        label,
        s.dark_mean,
        s.dark_max,
        s.dark_over[0],
        s.dark_over[1],
        s.dark_over[2],
        s.ink_ratio,
        s.color_mean,
        s.color_max,
        s.color_over32,
        fmt_point(s.centroid_ours),
        fmt_point(s.centroid_ref),
        delta,
        fmt_rgb(s.mean_color_ours),
        fmt_rgb(s.mean_color_ref),
        format!("{}/{}", s.blobs_ours, s.blobs_ref),
        format!(
            "{}/{}",
            s.blob_dist_ours.map_or("-".to_string(), |d| format!("{d:.1}")),
            s.blob_dist_ref.map_or("-".to_string(), |d| format!("{d:.1}"))
        ),
        notes,
        how,
        s.skipped_from_x
            .map_or(String::new(), |x| format!(" [x>={x} は基準に無いので除く]"))
    )
}

// ---------------- 節ごとの行の組み立て ----------------

/// 基準の画像に、ある列より右のインクが無ければ、その列から右を比べない（隠れて写らなかったクリックを外す）。
fn blank_band_from(reference: &[u8], from_x: u32) -> bool {
    (0..H)
        .all(|y| (from_x..W).all(|x| darkness(&reference[((y * W + x) * 3) as usize..][..3]) < 4.0))
}

struct Section {
    name: &'static str,
    cases: Vec<Case>,
    /// 組み立てられなかった行（理由つき）。
    skipped: Vec<String>,
}

/// キャンバス全体の書き出し（幅 1000・高さ 300）の PNG か（設定画面のスクリーンショットは大きさが違う）。
fn is_line_image(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 24];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head[12..16] == *b"IHDR"
        && u32::from_be_bytes([head[16], head[17], head[18], head[19]]) == W
        && u32::from_be_bytes([head[20], head[21], head[22], head[23]]) == H
}

fn sut_of(dir: &Path, row: &Row) -> Option<PathBuf> {
    (!row.file.is_empty()).then(|| dir.join(&row.file))
}

/// 色の変化の節: 線の画像の行（`file` が空で `screenshot` が `draw_`）に、同じ値の `.sut` の行を結ぶ（無ければ、先端の色の変化がオンの行に値を重ねる）。
fn color_section(dir: &Path) -> Option<Section> {
    let rows = read_manifest(dir)?;
    let base = rows.iter().find(|r| {
        !r.file.is_empty() && r.group == "color" && r.item_en.starts_with("Change brush tip color")
    })?;
    let interval_max = rows.iter().find_map(|r| {
        let at = r.note.find("max (")? + "max (".len();
        first_number(&r.note[at..])
    });
    let mut sec = Section {
        name: "色の変化",
        cases: Vec::new(),
        skipped: Vec::new(),
    };
    for r in rows
        .iter()
        .filter(|r| r.file.is_empty() && is_line_image(&dir.join(&r.screenshot)))
    {
        let reference = dir.join(&r.screenshot);
        if r.item_en.contains("Randomize") || r.item_en.contains('+') {
            sec.skipped.push(format!(
                "{}: ストロークごとに引く乱数の 5 本の短い線で、線の座標が manifest に無い",
                r.screenshot
            ));
            continue;
        }
        let Some(value) = first_number(&r.to) else {
            sec.skipped.push(format!("{}: 値が読めない", r.screenshot));
            continue;
        };
        let exact = rows.iter().find(|s| {
            !s.file.is_empty()
                && s.group == r.group
                && s.item_en == r.item_en
                && first_number(&s.to) == Some(value)
        });
        let mut setup = Setup {
            size: number_after(&r.note, "size"),
            main: color_after(&r.note, "main"),
            sub: color_after(&r.note, "sub"),
            ..Setup::default()
        };
        // 間隔（固定・直径に対する %）: 「interval fixed max」は最大の値（どれかの行の note に「max (数)」と書いてある）
        if let Some(at) = r.note.find("interval fixed") {
            let tail = r.note[at + "interval fixed".len()..].trim_start_matches([',', ' ']);
            setup.interval_pct = if tail.starts_with("max") {
                interval_max
            } else {
                first_number(&tail.chars().take(12).collect::<String>())
            };
        }
        let (sut, how) = match exact {
            Some(s) => (sut_of(dir, s).unwrap(), "同じ値の .sut"),
            None => {
                setup.tip_value = Some((r.item_en.clone(), value));
                (
                    sut_of(dir, base).unwrap(),
                    "先端の色の変化がオンの .sut に値を重ねた",
                )
            }
        };
        sec.cases.push(Case {
            label: r.screenshot.trim_end_matches(".png").to_string(),
            sut,
            reference,
            shape: Shape::Line {
                from: LINE_FROM,
                to: LINE_TO,
                pressure: (1.0, 1.0),
            },
            how: format!(
                "{how}; 直径{} 間隔{}%",
                opt(setup.size),
                opt(setup.interval_pct)
            ),
            setup,
            background: None,
        });
    }
    Some(sec)
}

/// 散布の点の節: 線でなく 8 か所のクリック。
fn spray_section(dir: &Path) -> Option<Section> {
    let rows = read_manifest(dir)?;
    let base = rows
        .iter()
        .find(|r| !r.file.is_empty() && r.group == "spray" && r.item_en == "Spraying effect")?;
    let mut sec = Section {
        name: "散布効果の点",
        cases: Vec::new(),
        skipped: Vec::new(),
    };
    for r in rows.iter().filter(|r| {
        r.file.is_empty() && r.group == "spray-dot" && is_line_image(&dir.join(&r.screenshot))
    }) {
        if matches!(r.to.trim(), "" | "-") {
            sec.skipped.push(format!(
                "{}: 設定の値を変えた行でなく、ペンの筆圧の確かめ",
                r.screenshot
            ));
            continue;
        }
        let item = r.item_en.trim_end_matches(" (dots)");
        let bar = r.to.contains("段階");
        let value = first_number(&r.to);
        let exact = rows.iter().find(|s| {
            !s.file.is_empty()
                && s.group == "spray"
                && s.item_en == item
                && s.to.contains("段階") == bar
                && first_number(&s.to).is_some()
                && first_number(&s.to) == value
        });
        // 筆圧つきの点・線は、行の隣に同じ名前の .sut がある（その行だけの設定）
        let own = dir.join(Path::new(&r.screenshot).with_extension("sut"));
        let pressure_clicks = parse_pressure_clicks(&r.note);
        let pressure_lines = parse_pressure_lines(&r.note);
        let mut setup = Setup {
            size: number_after(&r.note, "ブラシサイズ"),
            spray_size: number_after(&r.note, "粒子サイズ"),
            spray_density: number_after(&r.note, "粒子密度"),
            spray_bias: number_after(&r.note, "散布偏向"),
            spray_link: item.contains("link to brush size"),
            ..Setup::default()
        };
        // この行が変える項目の値は、同じ値の .sut があればそれを読ませ、無ければ行の値を重ねる
        let (sut, how) = match exact {
            _ if own.is_file() => (own, "この行の .sut"),
            Some(s) => {
                match item {
                    "Particle density" => setup.spray_density = None,
                    "Spray deviation" => setup.spray_bias = None,
                    "Particle size" => setup.spray_size = None,
                    _ => {}
                }
                (sut_of(dir, s).unwrap(), "同じ値の .sut")
            }
            None => {
                match item {
                    "Particle density" => setup.spray_density = value,
                    "Spray deviation" => setup.spray_bias = value,
                    "Particle size" => setup.spray_size = value,
                    _ => {}
                }
                (sut_of(dir, base).unwrap(), "散布オンの .sut に値を重ねた")
            }
        };
        sec.cases.push(Case {
            label: r.screenshot.trim_end_matches(".png").to_string(),
            sut,
            reference: dir.join(&r.screenshot),
            shape: if !pressure_lines.is_empty() {
                Shape::Many(pressure_lines)
            } else if !pressure_clicks.is_empty() {
                Shape::Nudged(pressure_clicks)
            } else {
                Shape::Clicks(CLICK_XS.iter().map(|x| (*x, CLICK_Y)).collect())
            },
            how: format!(
                "{how}; 直径{} 粒子{} 密度{} 偏向{}",
                opt(setup.size),
                opt(setup.spray_size),
                opt(setup.spray_density),
                opt(setup.spray_bias)
            ),
            setup,
            background: None,
        });
    }
    Some(sec)
}

/// 線の節（補正・デュアルブラシ）: manifest が `.sut` と画像を結んでいる。線の座標は note から読む。
fn paired_section(name: &'static str, dir: &Path) -> Option<Section> {
    let rows = read_manifest(dir)?;
    let mut sec = Section {
        name,
        cases: Vec::new(),
        skipped: Vec::new(),
    };
    let mut last_path: Option<(Vec<(f64, f64)>, String)> = None;
    for r in rows.iter().filter(|r| !r.file.is_empty()) {
        let stem = r.file.trim_end_matches(".sut");
        let reference = if r.screenshot.ends_with("_screen.png") || r.screenshot.is_empty() {
            dir.join(format!("{stem}.png"))
        } else {
            dir.join(&r.screenshot)
        };
        let ramp = r.to.contains('→');
        // 座標が書いていない行（「同じ折れ線」「ほかは基準のまま」）は、その前に読めた行の線を引き継ぐ
        let (points, inherited) = match parse_path(&r.note) {
            Some(p) => {
                last_path = Some((p.clone(), r.note.clone()));
                (p, false)
            }
            None if count_points(&r.note) > 0 => {
                sec.skipped.push(format!(
                    "{stem}: 点が矢印でつながっていない（始点が並ぶ筆圧つきの線で、折れ線・直線の型の外）"
                ));
                continue;
            }
            None => match &last_path {
                Some((p, _)) => (p.clone(), true),
                None => {
                    sec.skipped
                        .push(format!("{stem}: 線の座標が note から読めない"));
                    continue;
                }
            },
        };
        let path_note = last_path
            .as_ref()
            .map_or(r.note.clone(), |(_, n)| n.clone());
        let pressure = {
            let n = numbers(&r.to.replace('→', " "));
            if ramp && n.len() >= 2 {
                (n[n.len() - 2], n[n.len() - 1])
            } else {
                (1.0, 1.0)
            }
        };
        // Shift の直線か、筆圧が始めから終わりへ変わる線なら直線。そうでなければ折れ線をなぞった動き
        let shift = path_note.to_lowercase().contains("shift") || ramp;
        let shape = if shift && points.len() == 2 {
            Shape::Line {
                from: points[0],
                to: points[1],
                pressure,
            }
        } else {
            let seconds = number_after(&path_note, "報告された所要時間")
                .or_else(|| number_after(&path_note, "duration"))
                .unwrap_or(1.0);
            Shape::Path { points, seconds }
        };
        sec.cases.push(Case {
            label: stem.to_string(),
            sut: dir.join(&r.file),
            reference,
            how: format!(
                "{}{}",
                if matches!(shape, Shape::Line { .. }) {
                    "Shift の直線"
                } else {
                    "折れ線（等速）"
                },
                if inherited {
                    "（線は前の行から引き継ぎ）"
                } else {
                    ""
                }
            ),
            shape,
            setup: Setup::default(),
            background: None,
        });
    }
    Some(sec)
}

/// 下地混色の線の中心（y = 150 の行）の、帯ごとの RGB の差の平均（描いた − 基準の絶対値の、3 チャンネルの平均）。始めの 100 画素（荷が立ち上がる所）と、
/// 帯の境の前後 40 画素を除いた帯の中、全体。表の最後に足す。
fn mix_bands(ours: &[u8], reference: &[u8]) -> String {
    let at = |img: &[u8], x: u32| -> [f64; 3] {
        let i = ((150 * W + x) * 3) as usize;
        [
            f64::from(img[i]),
            f64::from(img[i + 1]),
            f64::from(img[i + 2]),
        ]
    };
    let diff = |x: u32| {
        let (a, b) = (at(ours, x), at(reference, x));
        (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f64>() / 3.0
    };
    let mean = |from: u32, to: u32| (from..to).map(diff).sum::<f64>() / f64::from(to - from);
    let start = mean(22, 122);
    // 帯: 白 0〜200・青 200〜400・黄 400〜600・赤 600〜800・灰 800〜960（線の終わりの丸い端を除く）
    let bands: Vec<String> = [(122, 160), (240, 360), (440, 560), (640, 760), (840, 940)]
        .iter()
        .map(|&(a, b)| format!("{:.1}", mean(a, b)))
        .collect();
    let overall = mean(22, 960);
    let probe = |x: u32| {
        let (a, b) = (at(ours, x), at(reference, x));
        format!(
            "{:.0},{:.0},{:.0}/{:.0},{:.0},{:.0}",
            a[0], a[1], a[2], b[0], b[1], b[2]
        )
    };
    format!(
        "線の中心の RGB の差 始め{:.1} 白{} 青{} 黄{} 赤{} 灰{} 全体{:.1} | 色(描/基) x=140 {} x=320 {} x=500 {} x=700 {} x=900 {}",
        start,
        bands[0],
        bands[1],
        bands[2],
        bands[3],
        bands[4],
        overall,
        probe(140),
        probe(320),
        probe(500),
        probe(700),
        probe(900)
    )
}

/// 下地混色の節: 帯を敷いた面へ、行ごとの `.sut` で同じ直線を描く。帯は、基準の画像の最上段の行（線にかからない）を下まで伸ばしたもの。
/// 大きさ・描く色・線の座標は、最初の行の note（「ブラシサイズN, 主色#RRGGBB, …線(x0,y0)->(x1,y1)」）から読む。
fn mix_section(dir: &Path) -> Option<Section> {
    let rows = read_manifest(dir)?;
    let base = rows
        .iter()
        .find(|r| !r.file.is_empty() && r.note.contains("ブラシサイズ"))?;
    let setup = Setup {
        size: number_after(&base.note, "ブラシサイズ"),
        main: color_after(&base.note, "主色"),
        ..Setup::default()
    };
    let points = parse_path(&base.note)?;
    let mut sec = Section {
        name: "下地混色",
        cases: Vec::new(),
        skipped: Vec::new(),
    };
    for r in rows.iter().filter(|r| !r.file.is_empty()) {
        let reference = dir.join(&r.screenshot);
        let Ok(image) = read_reference(&reference) else {
            sec.skipped
                .push(format!("{}: 基準の画像が読めない", r.file));
            continue;
        };
        let row0 = &image[..(W * 3) as usize];
        let background: Vec<u8> = (0..H).flat_map(|_| row0.iter().copied()).collect();
        sec.cases.push(Case {
            label: r.file.trim_end_matches(".sut").to_string(),
            sut: dir.join(&r.file),
            reference,
            shape: Shape::Line {
                from: points[0],
                to: points[1],
                pressure: (1.0, 1.0),
            },
            how: format!("帯の上の Shift の直線; 直径{}", opt(setup.size)),
            setup: setup.clone(),
            background: Some(background),
        });
    }
    Some(sec)
}

// ---------------- 実行 ----------------

/// 1 行を、取り込んだまま（`taper_off` が偽）と、入り抜きを 0 にして（真）描いて比べる。
fn run_case(
    case: &Case,
    out: Option<&Path>,
    section: &str,
    taper_off: bool,
) -> Result<String, String> {
    let set = import(&case.sut).map_err(|e| format!("取り込めない: {e:?}"))?;
    let imported = set.brushes.first().ok_or("ブラシが無い")?;
    let mut brush = imported.brush.clone();
    let notes = imported.unrepresented.len() + set.notes.len();
    apply_setup(&mut brush, &case.setup);
    let taper = (brush.assist.taper_in, brush.assist.taper_out);
    if taper_off {
        brush.assist.taper_in = 0.0;
        brush.assist.taper_out = 0.0;
    }
    let want_gap = case
        .setup
        .interval_pct
        .map(|p| p / 100.0 * brush.base.radius * 2.0);
    let reference = read_reference(&case.reference)?;
    let ours = render(
        &brush,
        &case.shape,
        want_gap.unwrap_or(0.0),
        case.background.as_deref(),
    )?;
    let centres = case.shape.centres();
    let skip = (!centres.is_empty() && blank_band_from(&reference, 850)).then_some(850);
    let (stats, diff) = compare(&ours, &reference, skip, &centres);
    let tag = if taper_off { " +入り抜き0" } else { "" };
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let name = format!(
            "{section}_{}{}.png",
            case.label,
            if taper_off { "_notaper" } else { "" }
        );
        write_gray_png(&dir.join(name), &diff)?;
    }
    let how = format!("{} 入り抜き{:.0}/{:.0}", case.how, taper.0, taper.1);
    let mut text = line(&format!("{}{tag}", case.label), &stats, notes, &how);
    if case.background.is_some() {
        text.push_str(&format!("\n    {}", mix_bands(&ours, &reference)));
    }
    Ok(text)
}

fn run_section(sec: &Section, out: Option<&Path>) {
    println!("\n== {} ({} 行) ==", sec.name, sec.cases.len());
    println!("{}", header());
    for case in &sec.cases {
        for taper_off in [false, true] {
            if taper_off && case.background.is_some() {
                continue;
            }
            match run_case(case, out, sec.name, taper_off) {
                Ok(l) => println!("{l}"),
                Err(e) => println!("{:<44} 失敗: {e}", case.label),
            }
        }
    }
    for s in &sec.skipped {
        println!("(比べなかった) {s}");
    }
}

fn env_dir(name: &str) -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os(name)?);
    dir.is_dir().then_some(dir)
}

fn out_dir() -> Option<PathBuf> {
    std::env::var_os("YOLU_CSP_DIFF_OUT").map(PathBuf::from)
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出しが要る（YOLU_CSP_COLOR_DIR）"]
fn color_change_lines_against_the_reference() {
    let Some(dir) = env_dir("YOLU_CSP_COLOR_DIR") else {
        return;
    };
    let sec = color_section(&dir).expect("manifest.csv と先端の色の変化の行");
    run_section(&sec, out_dir().as_deref());
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出しが要る（YOLU_CSP_DIFF_DIR）"]
fn spray_dots_against_the_reference() {
    let Some(dir) = env_dir("YOLU_CSP_DIFF_DIR") else {
        return;
    };
    let sec = spray_section(&dir.join("task4/spray")).expect("manifest.csv と散布の行");
    run_section(&sec, out_dir().as_deref());
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出しが要る（YOLU_CSP_DIFF_DIR）"]
fn color_mixing_lines_against_the_reference() {
    let Some(dir) = env_dir("YOLU_CSP_DIFF_DIR") else {
        return;
    };
    let sec = mix_section(&dir.join("task5/mix")).expect("manifest.csv と下地混色の行");
    run_section(&sec, out_dir().as_deref());
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出しが要る（YOLU_CSP_DIFF_DIR）"]
fn stabilizer_and_dual_lines_against_the_reference() {
    let Some(dir) = env_dir("YOLU_CSP_DIFF_DIR") else {
        return;
    };
    for (name, sub) in [
        ("補正", "task5/stab"),
        ("デュアルブラシ（1）", "task5/dual"),
        ("デュアルブラシ（2）", "task6/dual2"),
    ] {
        match paired_section(name, &dir.join(sub)) {
            Some(sec) => run_section(&sec, out_dir().as_deref()),
            None => println!("\n== {name} == manifest.csv が読めない"),
        }
    }
}

// ---------------- 仕組みの確かめ（データが無くても回る） ----------------

#[test]
fn csv_keeps_quotes_commas_and_newlines() {
    let rows = parse_csv("\u{feff}a,b,c\r\n\"x,1\",\"he said \"\"hi\"\"\",\"l1\nl2\"\n,,\n");
    assert_eq!(rows[0], ["a", "b", "c"]);
    assert_eq!(rows[1], ["x,1", "he said \"hi\"", "l1\nl2"]);
    assert_eq!(rows[2], ["", "", ""]);
}

#[test]
fn numbers_skip_the_bar_steps_and_read_signs() {
    assert_eq!(numbers("3段階バー中2つ点灯"), [2.0]);
    assert_eq!(numbers("-7 (左端) +5 2.5"), [-7.0, 5.0, 2.5]);
    assert_eq!(number_after("width 12, tone", "width"), Some(12.0));
    assert_eq!(number_after("width is", "width"), None);
    assert_eq!(
        color_after("tone #102030, edge #0a0b0c", "edge"),
        Some(Rgba8::new(10, 11, 12, 255))
    );
    assert_eq!(
        color_after("edge words, edge #123456", "edge"),
        Some(Rgba8::new(0x12, 0x34, 0x56, 255))
    );
}

#[test]
fn paths_are_read_from_arrows_and_hyphens_only() {
    assert_eq!(
        parse_path("path (10,20)→(30,40)→(50,20)、3 (1,2),(3,4)"),
        Some(vec![(10.0, 20.0), (30.0, 40.0), (50.0, 20.0)])
    );
    assert_eq!(
        parse_path("line (5,6)->(70,6) tail"),
        Some(vec![(5.0, 6.0), (70.0, 6.0)])
    );
    assert_eq!(parse_path("line (5,6)-(70,6)").map(|p| p.len()), Some(2));
    assert_eq!(parse_path("starts (1,2),(3,4),(5,6) end"), None);
    assert_eq!(count_points("starts (1,2),(3,4),(5,6) (not a point)"), 3);
}

#[test]
fn pressure_clicks_and_lines_are_read_from_the_note() {
    let clicks =
        parse_pressure_clicks("x 70・190 = 0.25、x 430 = 1.0、y 150。x 5→9 の線を y=1(筆圧0.5)");
    assert_eq!(
        clicks,
        [
            ((70.0, CLICK_Y), 0.25),
            ((190.0, CLICK_Y), 0.25),
            ((430.0, CLICK_Y), 1.0)
        ]
    );
    let lines = parse_pressure_lines("x 5→90 の線を y=60(筆圧0.2)、y=150(筆圧1.0)");
    assert_eq!(lines.len(), 2);
    assert!(matches!(
        lines[1],
        Shape::Line {
            from: (5.0, 150.0),
            to: (90.0, 150.0),
            pressure: (1.0, 1.0)
        }
    ));
    assert!(parse_pressure_lines("no arrows y=60(筆圧0.2)").is_empty());
}

#[test]
fn identical_images_have_no_difference_and_a_shift_shows_in_the_centroid() {
    let mut a = vec![255u8; (W * H * 3) as usize];
    for y in 100..120 {
        for x in 100..140 {
            let i = ((y * W + x) * 3) as usize;
            a[i..i + 3].copy_from_slice(&[0, 0, 0]);
        }
    }
    let (same, _) = compare(&a, &a, None, &[]);
    assert_eq!(same.dark_over, [0; 3]);
    assert_eq!(same.dark_mean, 0.0);
    assert!((same.ink_ratio - 1.0).abs() < 1e-12);
    // 右へ 5 画素ずらした画像: 重心の差が 5 になり、ずらした分の画素が差になる
    let mut b = vec![255u8; (W * H * 3) as usize];
    for y in 100..120 {
        for x in 105..145 {
            let i = ((y * W + x) * 3) as usize;
            b[i..i + 3].copy_from_slice(&[0, 0, 0]);
        }
    }
    let (moved, _) = compare(&b, &a, None, &[]);
    let (o, r) = (moved.centroid_ours.unwrap(), moved.centroid_ref.unwrap());
    assert!((o.0 - r.0 - 5.0).abs() < 1e-9 && (o.1 - r.1).abs() < 1e-9);
    assert_eq!(moved.dark_over[2], 2 * 5 * 20);
}

#[test]
fn a_straight_line_paints_on_white_in_top_down_rows() {
    use yolu_core::BrushSettings;
    let mut brush = Brush::from(BrushSettings {
        radius: 5.0,
        hardness: 1.0,
        spacing: 0.1,
        ..BrushSettings::default()
    });
    brush.base.color = Rgba8::new(0, 0, 0, 255);
    let shape = Shape::Line {
        from: (50.0, 50.0),
        to: (150.0, 50.0),
        pressure: (1.0, 1.0),
    };
    let px = render(&brush, &shape, 0.0, None).unwrap();
    // 上の行から数えて y = 50 の行の中ほどが黒く、上の隅は白
    assert!(darkness(&px[((50 * W + 100) * 3) as usize..][..3]) > 200.0);
    assert!(darkness(&px[..3]) < 1.0);
    assert!(darkness(&px[((250 * W + 100) * 3) as usize..][..3]) < 1.0);
}

#[test]
fn a_banded_background_is_painted_top_down_and_the_mixing_brush_reads_it() {
    use yolu_core::{BrushSettings, ColorMix, MixMode};
    // 左半分が青・右半分が白の下地（上の行から）
    let background: Vec<u8> = (0..H)
        .flat_map(|_| {
            (0..W).flat_map(|x| {
                if x < W / 2 {
                    [0u8, 0, 255]
                } else {
                    [255, 255, 255]
                }
            })
        })
        .collect();
    let mut brush = Brush::from(BrushSettings {
        radius: 10.0,
        hardness: 1.0,
        spacing: 0.1,
        ..BrushSettings::default()
    });
    brush.base.color = Rgba8::new(255, 0, 0, 255);
    brush.mix = ColorMix {
        mode: MixMode::Mix,
        paint: 0.5,
        density: 1.0,
        stretch: 0.0,
        ..ColorMix::default()
    };
    let shape = Shape::Line {
        from: (100.0, 150.0),
        to: (300.0, 150.0),
        pressure: (1.0, 1.0),
    };
    let image = render(&brush, &shape, 0.0, Some(&background)).unwrap();
    let at = |x: u32, y: u32| {
        let i = ((y * W + x) * 3) as usize;
        [image[i], image[i + 1], image[i + 2]]
    };
    // 線の外は下地のまま（上が青・右が白）。線の上は、青と赤の半々
    assert_eq!(at(200, 20), [0, 0, 255]);
    assert_eq!(at(800, 150), [255, 255, 255]);
    let mixed = at(200, 150);
    assert!(
        mixed[0] > 120 && mixed[0] < 135 && mixed[2] > 120 && mixed[2] < 135,
        "{mixed:?}"
    );
    // 帯ごとの差は、同じ画像どうしなら 0
    let text = mix_bands(&image, &image);
    assert!(
        text.contains("始め0.0 白0.0 青0.0 黄0.0 赤0.0 灰0.0 全体0.0"),
        "{text}"
    );
}

#[test]
fn without_the_data_folders_the_runs_do_nothing() {
    // 環境変数が無い（または指すフォルダが無い）ときは、何も読まず何も書かずに戻る
    assert!(env_dir("YOLU_CSP_NO_SUCH_VARIABLE").is_none());
}
