//! 取り込んだ CLIP STUDIO PAINT のブラシ（`.sut`）の水彩境界と、CLIP STUDIO PAINT が書き出した画像（透明の背景）との差を数で出す。
//!
//! データはリポジトリに入れない。置き場は環境変数 `YOLU_CSP_DIFF_DIR`（直下の `task5/edge` に `manifest.csv` と `.sut`・線の画像）。
//! 無ければ何もせずに通る。`cargo test -p yolu-io --test brushes csp_edge_compare:: -- --ignored --nocapture` で表が出る。
//! ブラシの名前・ファイル名・測った値はコードに写さない（行は manifest から読み、表にだけ出す）。線の座標・ブラシの大きさ・色は、
//! manifest の note（座標の書いてある行）から読む。
//!
//! 行ごとに 2 本描く: 「前」は水彩境界を外したブラシ（今までの取り込みと同じ線）、「後」は取り込んだブラシのまま。比べる数は、
//! 白の上に重ねた暗さ（255 − 輝度）の差の平均（全画素）と、線の縁（基準の画像で、透明の画素から 4 画素の内の、色のある画素）の
//! 暗さの差の平均・平均の色（白の上）・平均のアルファ。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yolu_core::glam::DVec2;
use yolu_core::{Brush, BrushSample, Document, Rect, Rgba8, RowOrder};
use yolu_io::brushes::import;

/// 書き出しのキャンバス（幅 1000・高さ 300）。
const W: u32 = 1000;
const H: u32 = 300;
/// 線の縁とみる、透明の画素からの距離（チェビシェフ）。
const RING: i64 = 4;

struct Row {
    file: String,
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
                note: get(&r, "note"),
            })
            .filter(|r| r.file.ends_with(".sut"))
            .collect(),
    )
}

/// 文字列の中の `(x,y)` の点を、出てくる順に。続く点が矢印（→・->）でつながっていれば同じ線（線は 2 点以上の組）。
fn parse_points(note: &str) -> Vec<Vec<(f64, f64)>> {
    let mut groups: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut rest = note;
    let mut linked = false;
    while let Some(open) = rest.find('(') {
        let before = &rest[..open];
        let close = match rest[open..].find(')') {
            Some(c) => open + c,
            None => break,
        };
        let inner = &rest[open + 1..close];
        let point = inner
            .split_once(',')
            .and_then(|(a, b)| Some((a.trim().parse().ok()?, b.trim().parse().ok()?)));
        let arrow = before.trim().chars().all(|c| matches!(c, '→' | '-' | '>'))
            && !before.trim().is_empty();
        if let Some(p) = point {
            if linked && arrow {
                groups.last_mut().expect("前の点").push(p);
            } else {
                groups.push(vec![p]);
            }
            linked = true;
        } else {
            linked = false;
        }
        rest = &rest[close + 1..];
    }
    groups
}

/// `key` の直後の数（空白をはさんでよい）。
fn number_after(s: &str, key: &str) -> Option<f64> {
    let at = s.find(key)? + key.len();
    let digits: String = s[at..]
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    digits.parse().ok()
}

/// `key` の直後の `#RRGGBB`。
fn color_after(s: &str, key: &str) -> Option<Rgba8> {
    let at = s.find(key)? + key.len();
    let hex = s[at..].trim_start().strip_prefix('#')?.get(..6)?;
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some(Rgba8::new((v >> 16) as u8, (v >> 8) as u8, v as u8, 255))
}

/// 描く物（基準の行の note から）: 直線（Shift の直線と同じ。筆圧 1）とクリックの点、ブラシの直径と色。
struct Scene {
    lines: Vec<((f64, f64), (f64, f64))>,
    clicks: Vec<(f64, f64)>,
    size: f64,
    color: Rgba8,
}

fn scene(rows: &[Row]) -> Option<Scene> {
    rows.iter().find_map(|r| {
        let groups = parse_points(&r.note);
        let lines: Vec<_> = groups
            .iter()
            .filter(|g| g.len() == 2)
            .map(|g| (g[0], g[1]))
            .collect();
        let clicks: Vec<_> = groups
            .iter()
            .filter(|g| g.len() == 1)
            .map(|g| g[0])
            .collect();
        let size = number_after(&r.note, "ブラシサイズ")?;
        let color = color_after(&r.note, "メイン色")?;
        (!lines.is_empty()).then_some(Scene {
            lines,
            clicks,
            size,
            color,
        })
    })
}

fn sample(p: (f64, f64)) -> BrushSample {
    // core は左下が原点
    BrushSample::new(p.0, f64::from(H) - p.1, 1.0, 0.0, DVec2::ZERO).expect("有限の点")
}

/// 描いた straight RGBA（上の行から）。
fn render(brush: &Brush, scene: &Scene) -> Result<Vec<u8>, String> {
    let e = |e: yolu_core::CoreError| format!("{e:?}");
    let mut doc = Document::new(W, H).map_err(e)?;
    let layer = doc.add_layer("L").map_err(e)?;
    for &(a, b) in &scene.lines {
        let mut s = doc.begin_brush_stroke(layer, brush).map_err(e)?;
        s.add_sample(&mut doc, sample(a)).map_err(e)?;
        s.add_sample(&mut doc, sample(b)).map_err(e)?;
        s.end_line(&mut doc).map_err(e)?;
        doc.end_stroke(s).map_err(e)?;
    }
    for &p in &scene.clicks {
        let mut s = doc.begin_brush_stroke(layer, brush).map_err(e)?;
        s.add_sample(&mut doc, sample(p)).map_err(e)?;
        doc.end_stroke(s).map_err(e)?;
    }
    let mut out = vec![0u8; (W * H * 4) as usize];
    doc.composite_into(
        yolu_core::Channel::Color,
        Rect::new(0, 0, W, H),
        &mut out,
        RowOrder::TopDown,
    )
    .map_err(e)?;
    Ok(out)
}

/// 基準の画像（straight RGBA、上の行から）。
fn read_reference(path: &Path) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("読めない: {e}"))?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{e}"))?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("大きさが数えられない")?];
    let info = reader.next_frame(&mut buf).map_err(|e| format!("{e}"))?;
    if (info.width, info.height) != (W, H) || info.color_type != png::ColorType::Rgba {
        return Err(format!(
            "{}x{} {:?} は読まない",
            info.width, info.height, info.color_type
        ));
    }
    Ok(buf[..info.buffer_size()].to_vec())
}

/// 白の上の色。
fn over_white(p: &[u8]) -> [f64; 3] {
    let a = f64::from(p[3]) / 255.0;
    [0, 1, 2].map(|c| f64::from(p[c]) * a + 255.0 * (1.0 - a))
}

fn darkness(rgb: [f64; 3]) -> f64 {
    255.0 - (0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2])
}

#[derive(Debug, Default)]
struct Stats {
    /// 暗さの差の平均（全画素）。
    dark_mean: f64,
    /// 縁の画素の暗さの差の平均・平均の色（描いた・基準）・平均のアルファ（描いた・基準）。
    ring_dark: f64,
    ring_color: ([f64; 3], [f64; 3]),
    ring_alpha: (f64, f64),
}

/// 基準の画像の縁（透明の画素から RING の内の、アルファのある画素）。
fn ring(reference: &[u8]) -> Vec<usize> {
    let alpha = |x: i64, y: i64| reference[((y * W as i64 + x) * 4 + 3) as usize];
    let mut out = Vec::new();
    for y in 0..H as i64 {
        for x in 0..W as i64 {
            if alpha(x, y) == 0 {
                continue;
            }
            let near = (-RING..=RING).any(|dy| {
                (-RING..=RING).any(|dx| {
                    let (nx, ny) = (x + dx, y + dy);
                    nx >= 0 && ny >= 0 && nx < W as i64 && ny < H as i64 && alpha(nx, ny) == 0
                })
            });
            if near {
                out.push((y * W as i64 + x) as usize);
            }
        }
    }
    out
}

fn compare(ours: &[u8], reference: &[u8], ring: &[usize]) -> Stats {
    let mut s = Stats::default();
    let n = (W * H) as usize;
    for i in 0..n {
        let (o, r) = (&ours[i * 4..i * 4 + 4], &reference[i * 4..i * 4 + 4]);
        s.dark_mean += (darkness(over_white(o)) - darkness(over_white(r))).abs();
    }
    s.dark_mean /= n as f64;
    for &i in ring {
        let (o, r) = (&ours[i * 4..i * 4 + 4], &reference[i * 4..i * 4 + 4]);
        let (wo, wr) = (over_white(o), over_white(r));
        s.ring_dark += (darkness(wo) - darkness(wr)).abs();
        for c in 0..3 {
            s.ring_color.0[c] += wo[c];
            s.ring_color.1[c] += wr[c];
        }
        s.ring_alpha.0 += f64::from(o[3]);
        s.ring_alpha.1 += f64::from(r[3]);
    }
    let k = ring.len().max(1) as f64;
    s.ring_dark /= k;
    s.ring_color = (s.ring_color.0.map(|v| v / k), s.ring_color.1.map(|v| v / k));
    s.ring_alpha = (s.ring_alpha.0 / k, s.ring_alpha.1 / k);
    s
}

fn rgb(c: [f64; 3]) -> String {
    format!("{:>3.0},{:>3.0},{:>3.0}", c[0], c[1], c[2])
}

fn run(dir: &Path) {
    let Some(rows) = read_manifest(dir) else {
        println!("manifest.csv が読めない");
        return;
    };
    let Some(scene) = scene(&rows) else {
        println!("線・ブラシサイズ・色を note から読めない");
        return;
    };
    println!(
        "\n== 水彩境界 ({} 行) ==\n{:<28} {:>9} {:>9} | {:>9} {:>9} | {:<13} {:<13} {:<13} | {:>6} {:>6} {:>6}",
        rows.len(),
        "行",
        "暗さ:前",
        "暗さ:後",
        "縁:前",
        "縁:後",
        "縁の色:前",
        "縁の色:後",
        "縁の色:基準",
        "α:前",
        "α:後",
        "α:基準",
    );
    for r in &rows {
        let stem = r.file.trim_end_matches(".sut");
        let result = (|| -> Result<String, String> {
            let set = import(&dir.join(&r.file)).map_err(|e| format!("取り込めない: {e:?}"))?;
            let imported = set.brushes.first().ok_or("ブラシが無い")?;
            let mut after = imported.brush.clone();
            after.base.radius = scene.size / 2.0;
            after.base.color = scene.color;
            let before = Brush {
                water_edge: None,
                ..after.clone()
            };
            let reference = read_reference(&dir.join(format!("{stem}.png")))?;
            let ring = ring(&reference);
            let b = compare(&render(&before, &scene)?, &reference, &ring);
            let a = compare(&render(&after, &scene)?, &reference, &ring);
            Ok(format!(
                "{:<28} {:>9.3} {:>9.3} | {:>9.2} {:>9.2} | {:<13} {:<13} {:<13} | {:>6.1} {:>6.1} {:>6.1}{}",
                stem,
                b.dark_mean,
                a.dark_mean,
                b.ring_dark,
                a.ring_dark,
                rgb(b.ring_color.0),
                rgb(a.ring_color.0),
                rgb(a.ring_color.1),
                b.ring_alpha.0,
                a.ring_alpha.0,
                a.ring_alpha.1,
                match after.water_edge {
                    Some(e) => format!(
                        "  （幅 {} 透明度 {} 明度 {} 後処理 {} ぼかし {}）",
                        e.width, e.opacity, e.darkness, e.after_stroke, e.blur
                    ),
                    None => "  （水彩境界なし）".into(),
                }
            ))
        })();
        match result {
            Ok(l) => println!("{l}"),
            Err(e) => println!("{stem:<28} 失敗: {e}"),
        }
    }
}

fn edge_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("YOLU_CSP_DIFF_DIR")?).join("task5/edge");
    dir.is_dir().then_some(dir)
}

#[test]
#[ignore = "CLIP STUDIO PAINT の書き出しが要る（YOLU_CSP_DIFF_DIR）"]
fn water_edges_against_the_reference() {
    let Some(dir) = edge_dir() else {
        println!("YOLU_CSP_DIFF_DIR の task5/edge が無いので比べない");
        return;
    };
    run(&dir);
}

#[test]
fn points_lines_and_settings_are_read_from_the_note() {
    let note = "線(10,20)→(30,20)と点(40,50)。ブラシサイズ12・メイン色#102030";
    let groups = parse_points(note);
    assert_eq!(
        groups,
        vec![vec![(10.0, 20.0), (30.0, 20.0)], vec![(40.0, 50.0)]]
    );
    let rows = [Row {
        file: "a.sut".into(),
        note: note.into(),
    }];
    let s = scene(&rows).expect("読める");
    assert_eq!(s.lines, vec![((10.0, 20.0), (30.0, 20.0))]);
    assert_eq!(s.clicks, vec![(40.0, 50.0)]);
    assert_eq!(s.size, 12.0);
    assert_eq!(s.color, Rgba8::new(0x10, 0x20, 0x30, 255));
    // 矢印の無い点は別の組
    assert_eq!(parse_points("(1,2) と (3,4)").len(), 2);
}

#[test]
fn a_ring_is_the_coloured_pixels_near_transparency() {
    let mut img = vec![0u8; (W * H * 4) as usize];
    // 20×20 の不透明の正方形
    for y in 100..120 {
        for x in 100..120 {
            img[((y * W + x) * 4 + 3) as usize] = 255;
        }
    }
    let r = ring(&img);
    // 内側の 12×12 は縁でない
    assert_eq!(r.len(), 20 * 20 - 12 * 12);
    let s = compare(&img, &img, &r);
    assert_eq!(s.dark_mean, 0.0);
    assert_eq!(s.ring_alpha, (255.0, 255.0));
}
