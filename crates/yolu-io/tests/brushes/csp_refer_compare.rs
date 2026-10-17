//! 取り込んだ CLIP STUDIO PAINT のブラシ（`.sut`）の「はみ出し防止」で描いた線と、CLIP STUDIO PAINT が書き出した画像との差を数で出す。
//!
//! データはリポジトリに入れない。置き場は環境変数で受ける（無ければ何もせずに通る）:
//! - `YOLU_CSP_DIFF_DIR`: 書き出しのフォルダ（直下の `task6/refer` に `manifest.csv`・`.sut`・画像）
//! - `YOLU_CSP_DIFF_OUT`: 差の画像（差の大きさを白黒に。白が差なし）を書くフォルダ（無ければ書かない）
//!
//! `cargo test -p yolu-io --test brushes csp_refer_compare:: -- --ignored --nocapture` で表が出る。行ごとに、はみ出し防止を外して描いた線（前）と、
//! 取り込んだとおりに描いた線（後）を比べる。参照レイヤーは、基準の画像から線の画素（灰色）を拾って作る（白の上の黒の不透明度として。
//! 線が赤に塗られた画像では拾えないので、線が見えている行の画像から作る）。描く線（点の列・秒）・大きさ・色は manifest の note から読み、
//! `.sut` の無い行は、note の名指す行の `.sut` を使う。ブラシの名前・ファイル名・値はコードに写さない。
//!
//! 比べ方は `csp_line_compare` と同じ（白の上に重ね、暗さの差と RGB の差）。加えて、参照レイヤーの線の画素のうち、線を描いた赤で
//! 塗られた画素の数を、基準と並べて出す。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushSample, Channel, Document, LayerId, Rect, Rgba8, RowOrder};
use yolu_io::brushes::import;

const W: u32 = 1000;
const H: u32 = 300;
/// 折れ線を、入力の列へ直す刻み（画素）。
const PATH_STEP: f64 = 4.0;

struct Row {
    file: String,
    screenshot: String,
    note: String,
}

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
                screenshot: get(&r, "screenshot"),
                note: get(&r, "note"),
            })
            .collect(),
    )
}

/// `key` の直後の数。
fn number_after(s: &str, key: &str) -> Option<f64> {
    let at = s.find(key)? + key.len();
    let rest = s[at..].trim_start_matches([' ', '=', ':']);
    let text: String = rest
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    text.parse().ok()
}

/// `key` の後の最初の `#RRGGBB`（key が空なら note の最初の色）。
fn color_after(s: &str, key: &str) -> Option<Rgba8> {
    let at = s.find(key)? + key.len();
    let rest = &s[at..];
    let hex = rest[rest.find('#')? + 1..].get(..6)?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgba8::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

/// 点（左上原点の x・y と筆圧）。
type Point = (f64, f64, f64);

/// `[[x,y],[x,y]]` か `[[x,y,p],…]` の点の列と、続く秒（`1.0s`）。左上原点。
fn parse_stroke(note: &str) -> Option<(Vec<Point>, f64)> {
    let start = note.find("[[")?;
    let end = start + note[start..].find("]]")? + 2;
    let inner = &note[start + 2..end - 2];
    let points: Vec<(f64, f64, f64)> = inner
        .split("],[")
        .filter_map(|p| {
            let v: Vec<f64> = p.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            match v.as_slice() {
                [x, y] => Some((*x, *y, 1.0)),
                [x, y, p] => Some((*x, *y, *p)),
                _ => None,
            }
        })
        .collect();
    let rest = note[end..].trim_start();
    let seconds: f64 = rest
        .split('s')
        .next()
        .and_then(|t| t.trim().parse().ok())
        .unwrap_or(1.0);
    (points.len() >= 2).then_some((points, seconds))
}

/// 基準の画像（上の行から、白の上に重ねた RGB）。
fn read_png(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("読めない: {e}"))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("大きさが数えられない")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{e}"))?;
    if (info.width, info.height) != (W, H) {
        return Err(format!("大きさが {}x{}", info.width, info.height));
    }
    let n = (W * H) as usize;
    let data = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => (0..n)
            .flat_map(|i| [data[i * 3], data[i * 3 + 1], data[i * 3 + 2], 255])
            .collect(),
        other => return Err(format!("{other:?} は読まない")),
    };
    Ok(over_white(&rgba))
}

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

/// 灰色（R・G・B がほぼ同じで白より暗い）の画素を、黒の不透明度にした参照レイヤー（上の行から）。
fn reference_alpha(rgb: &[u8]) -> Vec<u8> {
    rgb.as_chunks::<3>()
        .0
        .iter()
        .map(|p| {
            let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            if hi - lo <= 6 && hi < 250 {
                255 - hi
            } else {
                0
            }
        })
        .collect()
}

fn darkness(p: &[u8]) -> f64 {
    255.0 - (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
}

/// 赤で塗られた画素か（R が G・B より十分に大きい）。
fn red(p: &[u8]) -> bool {
    p[0] as i32 - p[1] as i32 > 60 && p[0] as i32 - p[2] as i32 > 60
}

struct Stats {
    dark_mean: f64,
    dark_over32: usize,
    color_over32: usize,
    /// 参照の線の画素のうち、赤で塗った数。
    line_painted: usize,
}

fn compare(ours: &[u8], reference: &[u8], line: &[u8]) -> (Stats, Vec<u8>) {
    let mut s = Stats {
        dark_mean: 0.0,
        dark_over32: 0,
        color_over32: 0,
        line_painted: 0,
    };
    let mut diff = vec![255u8; (W * H) as usize];
    for i in 0..(W * H) as usize {
        let (po, pr) = (&ours[i * 3..i * 3 + 3], &reference[i * 3..i * 3 + 3]);
        let d = (darkness(po) - darkness(pr)).abs();
        s.dark_mean += d;
        if d > 32.0 {
            s.dark_over32 += 1;
        }
        if (0..3).map(|c| po[c].abs_diff(pr[c])).max().unwrap_or(0) > 32 {
            s.color_over32 += 1;
        }
        if line[i] > 0 && red(po) {
            s.line_painted += 1;
        }
        diff[i] = 255 - d.round().clamp(0.0, 255.0) as u8;
    }
    s.dark_mean /= f64::from(W * H);
    (s, diff)
}

fn line_painted(rgb: &[u8], line: &[u8]) -> usize {
    (0..(W * H) as usize)
        .filter(|&i| line[i] > 0 && red(&rgb[i * 3..i * 3 + 3]))
        .count()
}

/// 参照レイヤー（上の行からの不透明度）と塗るレイヤーの文書に、点の列を 1 本のストロークで描き、白の上に重ねた RGB を返す。
fn render(
    brush: &Brush,
    line: &[u8],
    points: &[(f64, f64, f64)],
    seconds: f64,
) -> Result<Vec<u8>, String> {
    let e = |e: yolu_core::CoreError| format!("{e:?}");
    let mut doc = Document::new(W, H).map_err(e)?;
    let refer = doc.add_layer("参照").map_err(e)?;
    for y in 0..H {
        for x in 0..W {
            let a = line[(y * W + x) as usize];
            if a > 0 {
                doc.set_pixel(refer, x, H - 1 - y, Rgba8::new(0, 0, 0, a))
                    .map_err(e)?;
            }
        }
    }
    let paint: LayerId = doc.add_layer("塗る").map_err(e)?;
    let marked: HashSet<LayerId> = [refer].into_iter().collect();
    let mut stroke = doc.begin_brush_stroke(paint, brush).map_err(e)?;
    stroke
        .use_reference_layers(&mut doc, &marked, Channel::Color)
        .map_err(e)?;
    let mut lengths = vec![0.0];
    for w in points.windows(2) {
        lengths.push(lengths.last().unwrap() + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let total = *lengths.last().unwrap();
    let n = (total / PATH_STEP).ceil().max(1.0) as usize;
    for k in 0..=n {
        let d = total * k as f64 / n as f64;
        let seg = lengths
            .windows(2)
            .position(|w| d <= w[1])
            .unwrap_or(points.len() - 2);
        let span = (lengths[seg + 1] - lengths[seg]).max(1e-9);
        let t = ((d - lengths[seg]) / span).clamp(0.0, 1.0);
        let (a, b) = (points[seg], points[seg + 1]);
        let (x, y, p) = (
            a.0 + (b.0 - a.0) * t,
            a.1 + (b.1 - a.1) * t,
            a.2 + (b.2 - a.2) * t,
        );
        let sample = BrushSample::new(
            x,
            f64::from(H) - y,
            p,
            seconds * d / total.max(1e-9),
            DVec2::ZERO,
        )
        .map_err(e)?;
        stroke.add_sample(&mut doc, sample).map_err(e)?;
    }
    doc.end_stroke(stroke).map_err(e)?;
    let mut out = vec![0u8; (W * H * 4) as usize];
    doc.composite_into(
        Channel::Color,
        Rect::new(0, 0, W, H),
        &mut out,
        RowOrder::TopDown,
    )
    .map_err(e)?;
    Ok(over_white(&out))
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

fn run_row(
    dir: &Path,
    row: &Row,
    sut: &Path,
    line: &[u8],
    base_note: &str,
    out: Option<&Path>,
) -> Result<String, String> {
    let set = import(sut).map_err(|e| format!("取り込めない: {e:?}"))?;
    let imported = set.brushes.first().ok_or("ブラシが無い")?;
    let mut brush = imported.brush.clone();
    // 大きさと色は、その行の note に無ければ節の基準の行の note から（note は「ほかは基準と同じ」と書く）
    if let Some(size) = number_after(&row.note, "size").or_else(|| number_after(base_note, "size"))
    {
        brush.base.radius = size / 2.0;
    }
    brush.base.color = color_after(&row.note, "")
        .or_else(|| color_after(base_note, ""))
        .unwrap_or(Rgba8::new(0, 0, 0, 255));
    let (points, seconds) = parse_stroke(&row.note).ok_or("線が note に無い")?;
    let reference = read_png(&dir.join(&row.screenshot))?;
    let mut before_brush = brush.clone();
    before_brush.reference_lines.enabled = false;
    let before = render(&before_brush, line, &points, seconds)?;
    let after = render(&brush, line, &points, seconds)?;
    let (b, _) = compare(&before, &reference, line);
    let (a, diff) = compare(&after, &reference, line);
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let stem = Path::new(&row.screenshot)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        write_gray_png(&dir.join(format!("refer_{stem}.png")), &diff)?;
    }
    let l = &brush.reference_lines;
    Ok(format!(
        "{:<36} | {:>8.3} {:>8.3} | {:>7} {:>7} | {:>7} {:>7} | {:>5} {:>5} {:>5} | {} | 知らせ {}",
        row.screenshot,
        b.dark_mean,
        a.dark_mean,
        b.dark_over32,
        a.dark_over32,
        b.color_over32,
        a.color_over32,
        b.line_painted,
        a.line_painted,
        line_painted(&reference, line),
        if l.enabled {
            format!(
                "はみ出し防止 色の誤差 {} 拡縮 {} {} {:?}",
                l.margin, l.expand, l.amount, l.mode
            )
        } else {
            "はみ出し防止 切".into()
        },
        imported.unrepresented.len() + set.notes.len(),
    ))
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出し（YOLU_CSP_DIFF_DIR）が要る"]
fn reference_lines_against_clip_studio() {
    let Some(root) = std::env::var_os("YOLU_CSP_DIFF_DIR").map(PathBuf::from) else {
        eprintln!("YOLU_CSP_DIFF_DIR が無いので比べない");
        return;
    };
    let dir = root.join("task6").join("refer");
    let Some(rows) = read_manifest(&dir) else {
        eprintln!("{} に manifest.csv が無い", dir.display());
        return;
    };
    let out = std::env::var_os("YOLU_CSP_DIFF_OUT").map(PathBuf::from);
    // 参照レイヤーの線: 線が塗られていない画像（はみ出し防止が線を残した行）から拾う。拾った線の画素が一番多い画像を使う
    let mut line: Vec<u8> = Vec::new();
    for row in &rows {
        if let Ok(rgb) = read_png(&dir.join(&row.screenshot)) {
            let a = reference_alpha(&rgb);
            if a.iter().filter(|&&v| v > 0).count() > line.iter().filter(|&&v| v > 0).count() {
                line = a;
            }
        }
    }
    if line.is_empty() {
        eprintln!("線を拾える画像が無い");
        return;
    }
    println!(
        "{:<36} | {:>8} {:>8} | {:>7} {:>7} | {:>7} {:>7} | {:>5} {:>5} {:>5} | 取り込んだ設定",
        "行", "暗さ前", "暗さ後", "暗32前", "暗32後", "色32前", "色32後", "線前", "線後", "線基準"
    );
    // `.sut` の無い行（ペンで描いた確かめ）は、最初の行（節の基準）の `.sut` で描く
    let Some(first) = rows.iter().find(|r| !r.file.is_empty()) else {
        eprintln!(".sut が無い");
        return;
    };
    let base = dir.join(&first.file);
    for row in &rows {
        let sut = if row.file.is_empty() {
            base.clone()
        } else {
            dir.join(&row.file)
        };
        match run_row(&dir, row, &sut, &line, &first.note, out.as_deref()) {
            Ok(text) => println!("{text}"),
            Err(e) => println!("{:<36} | {e}", row.screenshot),
        }
    }
}
