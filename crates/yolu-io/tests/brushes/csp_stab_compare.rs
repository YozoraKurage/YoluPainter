//! 補正（手ぶれ補正・後補正・はらい）の線を、CLIP STUDIO PAINT が書き出した画像と比べる（式を合わせる外の基準）。取り込んだブラシで
//! 今の補正の式で描いた線（後）と、補正の拡張の前の取り込み（手ぶれ補正の強さを糸の長さへ 1 対 1、速度による調整・はらい・後補正なし）で
//! 描いた線（前）を、同じ入力で描いて並べる。
//!
//! データはリポジトリに入れない。置き場は環境変数で受ける（無ければ何もせずに通る）:
//! - `YOLU_CSP_DIFF_DIR`: 書き出しのフォルダ（直下に `task5/stab`。`manifest.csv` と `.sut`・線の画像）
//! - `YOLU_CSP_DIFF_OUT`: 差の画像（白が差なし）を書くフォルダ（無ければ書かない）
//! - `YOLU_CSP_STAB_SCAN`: 1 なら、後補正の強さ・手ぶれ補正の糸の長さ（速度による調整を外して）・はらいを振った行も出す（換算を決めるときの表）
//!
//! `cargo test -p yolu-io --test brushes csp_stab_compare:: -- --ignored --nocapture` で表が出る。ブラシの名前・ファイル名・設定の値は
//! コードに写さない（行は manifest から読み、表にだけ出す）。
//!
//! 入力: 折れ線の行（note に矢印でつないだ 3 点以上の座標）は、同じ折れ線の行のうち補正の無い行（手ぶれ補正 0・後補正 切）の画像の線の中心
//! （列ごとの暗さの重心）を入力の点にする（マウスの入力の揺れは座標の直線に無く、後補正が整える物が無いため）。時刻は等速で note の所要時間。
//! ペンの行（note に `points [x,y,筆圧]` の並びと始点の並び）は、点の間を 2 画素おきに刻み、筆圧と時刻を直線で補う。座標は左上が原点の値を、
//! core の左下が原点へ `高さ − y` で写す。
//!
//! 比べ方: 両方を白の上に重ね、「暗さ」（255 − 輝度）の差を出す。折れ線は、角の間の辺の中心の線のまっすぐさ（直線に当てはめた残りの二乗平均）
//! も出す。ペンの行は、筆圧を抜く所の列ごとのインクの量（列の暗さの和 ÷ 255。線の太さに近い）と、線の終わりの列を出す。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushSample, Document, PostCorrection, Rect, Rgba8, RowOrder};
use yolu_io::brushes::{import, SUT_STABILIZER_PER_LEVEL};

const W: u32 = 1000;
const H: u32 = 300;
/// ペンの線の点の間を刻む長さ（画素）。
const PEN_STEP: f64 = 2.0;

struct Row {
    file: String,
    note: String,
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (mut rows, mut row, mut field, mut quoted) = (Vec::new(), Vec::new(), String::new(), false);
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
                note: get(&r, "note"),
            })
            .filter(|r| r.file.ends_with(".sut"))
            .collect(),
    )
}

/// `(x,y)` を矢印でつないだ最初の列（3 点以上）。
fn parse_polyline(note: &str) -> Option<Vec<(f64, f64)>> {
    let mut out: Vec<(f64, f64)> = Vec::new();
    let mut rest = note;
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(')') else {
            break;
        };
        let point = after[..close].split_once(',').and_then(|(a, b)| {
            Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?))
        });
        let tail = &after[close + 1..];
        match point {
            Some(p) => {
                out.push(p);
                if !tail.starts_with('→') {
                    if out.len() >= 3 {
                        return Some(out);
                    }
                    out.clear();
                }
            }
            None => out.clear(),
        }
        rest = tail;
    }
    (out.len() >= 3).then_some(out)
}

/// `key` の直後の数。
fn number_after(s: &str, key: &str) -> Option<f64> {
    let at = s.find(key)? + key.len();
    let digits: String = s[at..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.trim_end_matches('.').parse().ok()
}

/// ペンの線の始点の並び・点の並び（始点からのずれと筆圧）・所要時間（秒）。
type PenLines = (Vec<(f64, f64)>, Vec<(f64, f64, f64)>, f64);

/// ペンの線: `[x(+dx),y,筆圧]` の並びと、`(x,y)` の始点の並び。始点ごとに 1 本。
fn parse_pen(note: &str) -> Option<PenLines> {
    let mut offsets = Vec::new();
    let mut rest = note;
    while let Some(open) = rest.find("[x") {
        let after = &rest[open + 2..];
        let close = after.find(']')?;
        let inner = &after[..close];
        let mut parts = inner.split(',');
        let dx = parts.next()?.trim();
        let dx = if dx.is_empty() {
            0.0
        } else {
            dx.trim_start_matches('+').parse().ok()?
        };
        let dy_part = parts.next()?.trim();
        let dy = dy_part
            .strip_prefix('y')
            .map(|d| {
                if d.is_empty() {
                    Some(0.0)
                } else {
                    d.trim_start_matches('+').parse().ok()
                }
            })
            .unwrap_or(None)?;
        let pressure: f64 = parts.next()?.trim().parse().ok()?;
        offsets.push((dx, dy, pressure));
        rest = &after[close..];
    }
    if offsets.len() < 2 {
        return None;
    }
    let seconds = number_after(note, "duration")?;
    let starts_at = note.find("座標")?;
    let mut starts = Vec::new();
    let mut rest = &note[starts_at..];
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(')') else {
            break;
        };
        if let Some(p) = after[..close]
            .split_once(',')
            .and_then(|(a, b)| Some((a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?)))
        {
            starts.push(p);
        }
        rest = &after[close + 1..];
    }
    (!starts.is_empty()).then_some((starts, offsets, seconds))
}

fn sample(x: f64, y: f64, pressure: f64, time: f64) -> BrushSample {
    BrushSample::new(x, f64::from(H) - y, pressure, time, DVec2::ZERO).expect("有限の入力")
}

/// 線の中心（列ごとの暗さの重心。左上原点）を、インクのある列ごとに 1 点。
fn centerline(rgb: &[u8], from_x: f64, to_x: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for x in from_x.floor() as u32..=(to_x.ceil() as u32).min(W - 1) {
        let (mut sum, mut wy) = (0.0, 0.0);
        for y in 0..H {
            let d = darkness(&rgb[((y * W + x) * 3) as usize..][..3]);
            sum += d;
            wy += d * (f64::from(y) + 0.5);
        }
        if sum > 255.0 {
            out.push((f64::from(x) + 0.5, wy / sum));
        }
    }
    out
}

/// 点の列を等速（全体で `seconds`）の入力に。
fn timed(points: &[(f64, f64)], seconds: f64) -> Vec<BrushSample> {
    let mut lengths = vec![0.0];
    for w in points.windows(2) {
        lengths.push(lengths.last().unwrap() + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let total = lengths.last().copied().unwrap_or(0.0).max(1e-9);
    points
        .iter()
        .zip(&lengths)
        .map(|(p, l)| sample(p.0, p.1, 1.0, seconds * l / total))
        .collect()
}

/// ペンの線 1 本の入力（点の間を刻む）。
fn pen_line(start: (f64, f64), offsets: &[(f64, f64, f64)], seconds: f64) -> Vec<BrushSample> {
    let points: Vec<(f64, f64, f64)> = offsets
        .iter()
        .map(|(dx, dy, p)| (start.0 + dx, start.1 + dy, *p))
        .collect();
    let mut lengths = vec![0.0];
    for w in points.windows(2) {
        lengths.push(lengths.last().unwrap() + (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1));
    }
    let total = lengths.last().copied().unwrap_or(0.0).max(1e-9);
    let mut out = vec![sample(points[0].0, points[0].1, points[0].2, 0.0)];
    for (k, w) in points.windows(2).enumerate() {
        let len = lengths[k + 1] - lengths[k];
        let n = (len / PEN_STEP).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let at = lengths[k] + len * t;
            out.push(sample(
                w[0].0 + (w[1].0 - w[0].0) * t,
                w[0].1 + (w[1].1 - w[0].1) * t,
                w[0].2 + (w[1].2 - w[0].2) * t,
                seconds * at / total,
            ));
        }
    }
    out
}

/// ブラシでストロークを順に描き、白の上に重ねた RGB（上の行から）を返す。
fn render(brush: &Brush, strokes: &[Vec<BrushSample>]) -> Result<Vec<u8>, String> {
    let mut doc = Document::new(W, H).map_err(|e| format!("{e:?}"))?;
    let layer = doc.add_layer("L").map_err(|e| format!("{e:?}"))?;
    for points in strokes {
        let mut stroke = doc
            .begin_brush_stroke(layer, brush)
            .map_err(|e| format!("{e:?}"))?;
        for p in points {
            stroke
                .add_sample(&mut doc, *p)
                .map_err(|e| format!("{e:?}"))?;
        }
        doc.end_stroke(stroke).map_err(|e| format!("{e:?}"))?;
    }
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    doc.composite_into(
        yolu_core::Channel::Color,
        Rect::new(0, 0, W, H),
        &mut rgba,
        RowOrder::TopDown,
    )
    .map_err(|e| format!("{e:?}"))?;
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
        other => return Err(format!("{other:?} は読まない")),
    };
    Ok(over_white(&rgba))
}

fn darkness(p: &[u8]) -> f64 {
    255.0 - (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]))
}

/// 暗さの差の平均（どちらかにインクのある画素）と、32 を超えた画素の数と、差の画像。
fn difference(ours: &[u8], reference: &[u8]) -> (f64, usize, Vec<u8>) {
    let (mut sum, mut n, mut over) = (0.0, 0usize, 0usize);
    let mut image = vec![255u8; (W * H) as usize];
    for i in 0..(W * H) as usize {
        let (a, b) = (
            darkness(&ours[i * 3..i * 3 + 3]),
            darkness(&reference[i * 3..i * 3 + 3]),
        );
        let d = (a - b).abs();
        image[i] = 255 - d.round().clamp(0.0, 255.0) as u8;
        if a > 8.0 || b > 8.0 {
            sum += d;
            n += 1;
        }
        if d > 32.0 {
            over += 1;
        }
    }
    (sum / n.max(1) as f64, over, image)
}

/// 折れ線の角の間の辺（角から 15 画素ずつ内側）の、中心の線を直線に当てはめた残りの二乗平均の平均。
fn straightness(rgb: &[u8], corners: &[(f64, f64)]) -> f64 {
    let mut total = 0.0;
    let mut sides = 0;
    for w in corners.windows(2) {
        let line = centerline(rgb, w[0].0 + 15.0, w[1].0 - 15.0);
        if line.len() < 4 {
            continue;
        }
        let n = line.len() as f64;
        let (mx, my) = (
            line.iter().map(|p| p.0).sum::<f64>() / n,
            line.iter().map(|p| p.1).sum::<f64>() / n,
        );
        let sxy: f64 = line.iter().map(|p| (p.0 - mx) * (p.1 - my)).sum();
        let sxx: f64 = line.iter().map(|p| (p.0 - mx) * (p.0 - mx)).sum();
        let k = sxy / sxx.max(1e-9);
        let r: f64 = line
            .iter()
            .map(|p| (p.1 - (my + k * (p.0 - mx))).powi(2))
            .sum::<f64>()
            / n;
        total += r.sqrt();
        sides += 1;
    }
    total / f64::from(sides.max(1))
}

/// 横の線（y の上下 10 画素）の、列 x のインクの量（暗さの和 ÷ 255）。
fn column_ink(rgb: &[u8], x: u32, y: f64) -> f64 {
    let y = y.round() as i64;
    (y - 10..=y + 10)
        .filter(|v| (0..i64::from(H)).contains(v))
        .map(|v| darkness(&rgb[((v as u32 * W + x) * 3) as usize..][..3]))
        .sum::<f64>()
        / 255.0
}

/// 横の線の終わり（インクが 0.5 を超える最後の列）。
fn line_end(rgb: &[u8], y: f64) -> u32 {
    (0..W)
        .rev()
        .find(|&x| column_ink(rgb, x, y) > 0.5)
        .unwrap_or(0)
}

/// 補正の拡張の前の取り込み（手ぶれ補正の強さを糸の長さへ 1 対 1、速度による調整・はらい・後補正なし）。
fn before(brush: &Brush) -> Brush {
    let mut b = brush.clone();
    b.assist.stabilizer = brush.assist.stabilizer / SUT_STABILIZER_PER_LEVEL;
    b.assist.stabilizer_speed = None;
    b.assist.sweep = 0.0;
    b.assist.post = PostCorrection::default();
    b
}

enum Kind {
    Path { corners: Vec<(f64, f64)> },
    Pen { ys: Vec<f64>, release: (f64, f64) },
}

struct Case {
    label: String,
    brush: Brush,
    strokes: Vec<Vec<BrushSample>>,
    reference: Vec<u8>,
    kind: Kind,
}

/// 折れ線と、その折れ線の補正の無い行の画像。
type Base = (Vec<(f64, f64)>, Vec<u8>);

fn cases(dir: &Path) -> Result<Vec<Case>, String> {
    let rows = read_manifest(dir).ok_or("manifest.csv が読めない")?;
    let mut out = Vec::new();
    // 折れ線ごとの、補正の無い行の画像（入力の点の元）
    let mut bases: Vec<Base> = Vec::new();
    let mut loaded = Vec::new();
    let mut last_path: Option<Vec<(f64, f64)>> = None;
    for r in &rows {
        let stem = r.file.trim_end_matches(".sut").to_string();
        let set = import(&dir.join(&r.file)).map_err(|e| format!("{stem}: 取り込めない: {e:?}"))?;
        let mut brush = set.brushes.first().ok_or("ブラシが無い")?.brush.clone();
        // 描いた色（note の黒。`.sut` は描画色を持たない）
        brush.base.color = Rgba8::new(0, 0, 0, 255);
        let reference = read_reference(&dir.join(format!("{stem}.png")))?;
        // 座標の無い折れ線の行（「同じ折れ線」）は、前に読めた行の折れ線を引き継ぐ
        let path = match parse_polyline(&r.note) {
            Some(p) => {
                last_path = Some(p.clone());
                Some(p)
            }
            None if r.note.contains("折れ線") && parse_pen(&r.note).is_none() => {
                last_path.clone()
            }
            None => None,
        };
        if let Some(path) = &path {
            let plain = brush.assist.stabilizer == 0.0 && !brush.assist.post.on;
            if plain && !bases.iter().any(|(p, _)| p == path) {
                bases.push((path.clone(), reference.clone()));
            }
        }
        loaded.push((stem, r, brush, reference, path));
    }
    for (stem, r, brush, reference, path) in loaded {
        if let Some(path) = path {
            let Some((_, base)) = bases.iter().find(|(p, _)| *p == path) else {
                continue;
            };
            let monotone = path.windows(2).all(|w| w[1].0 > w[0].0);
            if !monotone {
                continue;
            }
            let seconds = number_after(&r.note, "所要時間").unwrap_or(1.5);
            let line = centerline(base, path[0].0, path[path.len() - 1].0);
            out.push(Case {
                label: stem,
                brush,
                strokes: vec![timed(&line, seconds)],
                reference,
                kind: Kind::Path { corners: path },
            });
        } else if let Some((starts, offsets, seconds)) = parse_pen(&r.note) {
            let ys = starts.iter().map(|s| s.1).collect();
            // 筆圧を抜く所: 筆圧が下がり始める点から終わりまで（最初の線の x）
            let drop = offsets
                .windows(2)
                .position(|w| w[1].2 < w[0].2)
                .unwrap_or(0);
            let release = (
                starts[0].0 + offsets[drop].0,
                starts[0].0 + offsets[offsets.len() - 1].0,
            );
            out.push(Case {
                label: stem,
                brush,
                strokes: starts
                    .iter()
                    .map(|s| pen_line(*s, &offsets, seconds))
                    .collect(),
                reference,
                kind: Kind::Pen { ys, release },
            });
        }
    }
    Ok(out)
}

fn describe(case: &Case, rgb: &[u8]) -> String {
    match &case.kind {
        Kind::Path { corners } => format!("まっすぐさ {:.3}", straightness(rgb, corners)),
        Kind::Pen { ys, release } => {
            let y = ys[ys.len() / 2];
            let (a, b) = *release;
            let at = |f: f64| column_ink(rgb, (a + (b - a) * f).round() as u32, y);
            format!(
                "抜きの太さ {:.1}/{:.1}/{:.1}/{:.1} 終わり x {}",
                at(0.25),
                at(0.5),
                at(0.75),
                at(0.95),
                line_end(rgb, y)
            )
        }
    }
}

fn run(case: &Case, label: &str, brush: &Brush, out: Option<&Path>) -> String {
    match render(brush, &case.strokes) {
        Ok(ours) => {
            let (mean, over, image) = difference(&ours, &case.reference);
            if let Some(dir) = out {
                let _ = std::fs::create_dir_all(dir);
                let _ = write_gray_png(
                    &dir.join(format!("stab_{}_{label}_diff.png", case.label)),
                    &image,
                );
                let drawn: Vec<u8> = ours
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|p| 255 - darkness(p).round() as u8)
                    .collect();
                let _ = write_gray_png(
                    &dir.join(format!("stab_{}_{label}.png", case.label)),
                    &drawn,
                );
            }
            format!(
                "{:<28} {:<10} 暗さの差 {:>6.2} >32 {:>6} | {}",
                case.label,
                label,
                mean,
                over,
                describe(case, &ours)
            )
        }
        Err(e) => format!("{:<28} {label}: 失敗 {e}", case.label),
    }
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

fn stab_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("YOLU_CSP_DIFF_DIR")?).join("task5/stab");
    dir.is_dir().then_some(dir)
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出しが要る（YOLU_CSP_DIFF_DIR）"]
fn stabilization_lines_against_the_reference() {
    let Some(dir) = stab_dir() else {
        eprintln!("YOLU_CSP_DIFF_DIR/task5/stab が無いので比べない");
        return;
    };
    let out = std::env::var_os("YOLU_CSP_DIFF_OUT").map(PathBuf::from);
    let scan = std::env::var_os("YOLU_CSP_STAB_SCAN").is_some_and(|v| v == "1");
    let cases = match cases(&dir) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    println!("\n== 補正 ({} 行) ==", cases.len());
    for case in &cases {
        println!("{}", run(case, "前", &before(&case.brush), out.as_deref()));
        println!("{}", run(case, "後", &case.brush, out.as_deref()));
        let reference = describe(case, &case.reference);
        println!("{:<28} {:<10} {:>34} | {reference}", case.label, "基準", "");
        if !scan {
            continue;
        }
        let a = case.brush.assist;
        if a.post.on {
            for k in [0.25, 0.5, 2.0, 4.0, 8.0] {
                let mut b = case.brush.clone();
                b.assist.post.strength = (a.post.strength * k).min(100.0);
                println!("{}", run(case, &format!("後補正×{k}"), &b, out.as_deref()));
            }
        }
        if a.stabilizer > 0.0 {
            for length in [0.0, 1.0, 2.0, 3.0, 5.0, 8.0, 12.0, 20.0, 30.0, 50.0] {
                // 速度による調整を外して、糸の長さだけを振る（強さの換算を、速さの効きと分けて見る）
                let mut b = case.brush.clone();
                b.assist.stabilizer = length;
                b.assist.stabilizer_speed = None;
                println!("{}", run(case, &format!("糸{length}"), &b, None));
            }
        }
        if matches!(case.kind, Kind::Pen { .. }) {
            for sweep in [0.0, 0.25, 0.5, 0.75, 1.0] {
                let mut b = case.brush.clone();
                b.assist.sweep = sweep;
                println!("{}", run(case, &format!("はらい{sweep}"), &b, None));
            }
        }
    }
}

#[test]
fn notes_give_the_polyline_and_the_pen_lines() {
    let note = "折れ線 (10,20)→(30,40)→(50,20)、ほかは (1,2) の点";
    assert_eq!(
        parse_polyline(note),
        Some(vec![(10.0, 20.0), (30.0, 40.0), (50.0, 20.0)])
    );
    assert_eq!(parse_polyline("(1,2)→(3,4) だけ"), None);
    let pen = "各線: points [x,y,1.0],[x+150,y,1.0],[x+250,y,0.0]、duration 0.25s)。キャンバス座標 (100,70),(100,150) の2本";
    let (starts, offsets, seconds) = parse_pen(pen).unwrap();
    assert_eq!(starts, vec![(100.0, 70.0), (100.0, 150.0)]);
    assert_eq!(
        offsets,
        vec![(0.0, 0.0, 1.0), (150.0, 0.0, 1.0), (250.0, 0.0, 0.0)]
    );
    assert_eq!(seconds, 0.25);
    let line = pen_line(starts[0], &offsets, seconds);
    assert_eq!(line.len(), 1 + 75 + 50);
    let last = line.last().unwrap();
    assert_eq!((last.x, last.pressure, last.time), (350.0, 0.0, 0.25));
}

#[test]
fn without_the_data_folder_the_comparison_does_nothing() {
    if std::env::var_os("YOLU_CSP_DIFF_DIR").is_none() {
        assert!(stab_dir().is_none());
    }
}
