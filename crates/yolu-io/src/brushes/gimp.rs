//! GIMP のブラシ: `.gbr`（画像の筆先）、`.gih`（画像ホース。複数の筆先）、`.vbr`（パラメトリック）。
//!
//! GIMP の `devel-docs`（gbr.txt・gih.txt・vbr.txt）の形式の説明から書いた。GIMP のコードは使っていない。GIMP にあって
//! このエンジンに無い振る舞い（セルを筆圧などで選ぶ、色つきの筆）は、近似して `Unrepresented` で知らせる。

use std::collections::HashMap;
use std::sync::Arc;

use yolu_core::{Brush, BrushSettings, BrushTip, TipSelection};

use super::error::{BrushImportError, Counted, Fault, SizedItem};
use super::notes::{Source, Unrepresented, VbrShape};
use super::reader::{Budget, Reader};
use super::{short_text, to_byte, ImportedBrush};

const MAGIC: u32 = 0x4749_4D50; // 'GIMP'

/// 筆先を 1 枚読む（`.gbr` の中身。`.gih` のセルも同じ）。
struct Cell {
    tip: BrushTip,
    spacing_percent: f64,
    notes: Vec<Unrepresented>,
}

fn read_one_gbr(
    r: &mut Reader,
    fallback: Option<&str>,
    budget: &mut Budget,
) -> Result<Cell, Fault> {
    let mut notes = Vec::new();
    let start = r.position();
    let (header_size, version, width, height, bytes) =
        (r.u32()?, r.u32()?, r.u32()?, r.u32()?, r.u32()?);
    let side = BrushTip::MAX_SIZE;
    if width < 1 || height < 1 || width > side || height > side {
        return Err(Fault::SizeOutOfRange {
            what: SizedItem::Brush,
            width: width as i64,
            height: height as i64,
        });
    }
    let (spacing, name);
    match version {
        1 => {
            if header_size < 20 {
                return Err(Fault::GbrHeaderSize {
                    version,
                    size: header_size,
                });
            }
            spacing = 25.0;
            let n = r.count(header_size - 20, 1, Counted::NameLength)?;
            name = tip_name(r.bytes(n)?, fallback);
        }
        2 | 3 => {
            if header_size < 28 {
                return Err(Fault::GbrHeaderSize {
                    version,
                    size: header_size,
                });
            }
            if r.u32()? != MAGIC {
                return Err(Fault::GbrSignature);
            }
            spacing = r.u32()? as f64;
            if header_size - 28 > 256 {
                return Err(Fault::GbrNameTooLong);
            }
            name = tip_name(r.bytes((header_size - 28) as usize)?, fallback);
        }
        other => return Err(Fault::GbrVersion(other)),
    }
    if r.position() != start + header_size as usize {
        return Err(Fault::GbrHeaderMismatch);
    }
    if bytes != 1 && bytes != 4 {
        return Err(if bytes == 18 {
            Fault::GbrCinePaint
        } else {
            Fault::GbrPixelSize(bytes)
        });
    }
    let pixel_count = width * height;
    let n = r.count(pixel_count, bytes as usize, Counted::PixelData)?;
    budget.take(pixel_count as u64)?;
    let pixels = r.bytes(n * bytes as usize)?;
    let (w, h) = (width as usize, height as usize);
    let mut alpha = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            // ファイルの行は上から。筆先は下の行が先
            alpha[(h - 1 - y) * w + x] = if bytes == 1 {
                pixels[y * w + x]
            } else {
                pixels[(y * w + x) * 4 + 3]
            };
        }
    }
    if bytes == 4 {
        notes.push(Unrepresented::ColorTipAsMask);
    }
    let tip = BrushTip::new(&name, width, height, alpha).map_err(|_| Fault::SizeOutOfRange {
        what: SizedItem::Brush,
        width: width as i64,
        height: height as i64,
    })?;
    Ok(Cell {
        tip,
        spacing_percent: spacing,
        notes,
    })
}

/// 名前（最初の NUL まで、UTF-8）。空なら代わりの名前。
fn tip_name(bytes: &[u8], fallback: Option<&str>) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let name = short_text(&String::from_utf8_lossy(&bytes[..end]), 128);
    if name.is_empty() {
        fallback.unwrap_or("Untitled").to_string()
    } else {
        name
    }
}

/// GIMP の間隔は筆先の幅に対する %。このエンジンの間隔は直径（長い辺）に対する割合なので直す。
fn settings(tips: Vec<BrushTip>, spacing_percent: f64) -> Brush {
    let (w, h) = (tips[0].width() as f64, tips[0].height() as f64);
    let longest = w.max(h);
    let spacing = spacing_percent / 100.0 * w / longest;
    let mut brush = Brush {
        base: BrushSettings {
            radius: (longest / 2.0).max(0.5),
            spacing: spacing.clamp(0.01, 4.0),
            ..BrushSettings::default()
        },
        ..Brush::default()
    };
    if tips.len() == 1 {
        brush.tip.image = tips.into_iter().next().map(Arc::new);
    } else {
        brush.tip.images = tips.into_iter().map(Arc::new).collect();
    }
    brush
}

/// `.gbr`。1 つのブラシ。
pub(crate) fn read_gbr(
    data: &[u8],
    fallback: Option<&str>,
    budget: &mut Budget,
) -> Result<ImportedBrush, BrushImportError> {
    let mut r = Reader::new(data);
    let cell = read_one_gbr(&mut r, fallback, budget)?;
    let mut notes = cell.notes;
    if r.remaining() > 0 {
        notes.push(Unrepresented::GbrTrailingData {
            bytes: r.remaining(),
        });
    }
    let name = cell.tip.name().to_string();
    ImportedBrush::new(
        &name,
        Source::GimpGbr,
        settings(vec![cell.tip], cell.spacing_percent),
        notes,
    )
}

/// `.gih`。画像ホースは複数の筆先を持つ 1 つのブラシ。筆圧・角度・速さ・傾きでセルを選ぶ設定は、ランダムか順番で近似して知らせる。
pub(crate) fn read_gih(
    data: &[u8],
    fallback: Option<&str>,
    budget: &mut Budget,
) -> Result<ImportedBrush, BrushImportError> {
    let mut position = 0usize;
    let name = line(data, &mut position)?;
    let parameters = line(data, &mut position)?;
    let fields: Vec<&str> = parameters
        .split([' ', '\t'])
        .filter(|f| !f.is_empty())
        .collect();
    let count: u32 = match fields.first().and_then(|f| f.parse::<i32>().ok()) {
        Some(n) if (1..=256).contains(&n) => n as u32,
        _ => return Err(Fault::GihCellCount.into()),
    };
    let mut keys: HashMap<&str, &str> = HashMap::new();
    for field in fields.iter().skip(1) {
        if let Some(colon) = field.find(':') {
            if colon > 0 {
                keys.insert(&field[..colon], &field[colon + 1..]);
            }
        }
    }
    let mut notes = Vec::new();
    let mode = keys
        .get("sel0")
        .copied()
        .unwrap_or(if keys.contains_key("dim") {
            "random"
        } else {
            "incremental"
        });
    let mut selection = TipSelection::Random;
    if mode == "incremental" {
        selection = TipSelection::Sequential;
    } else if mode != "random" {
        notes.push(Unrepresented::HoseSelection {
            mode: short_text(mode, 32),
        });
    }
    if let Some(dim) = keys.get("dim") {
        if *dim != "1" {
            notes.push(Unrepresented::HoseDimensions {
                dim: short_text(dim, 32),
            });
        }
    }
    let mut r = Reader::at(data, position);
    let hose_name = short_text(&name, 128);
    let cell_fallback = if hose_name.is_empty() {
        fallback
    } else {
        Some(hose_name.as_str())
    };
    let mut tips = Vec::new();
    let mut spacing = 25.0;
    for i in 0..count {
        if i > 0 && r.remaining() == 0 {
            // ちょうどファイルの終わりでセルが尽きた（実在する配布物にある）。読めたセルで使い、そう伝える。
            // セルの途中で切れているものは read_one_gbr が断る。
            notes.push(Unrepresented::HoseShort {
                declared: count,
                present: i,
            });
            break;
        }
        let cell = read_one_gbr(&mut r, cell_fallback, budget)?;
        if i == 0 {
            spacing = cell.spacing_percent;
        }
        for n in cell.notes {
            if !notes.contains(&n) {
                notes.push(n);
            }
        }
        tips.push(cell.tip);
    }
    let mut brush = settings(tips, spacing);
    brush.tip.selection = selection;
    let name = if hose_name.is_empty() {
        fallback.unwrap_or("").to_string()
    } else {
        hose_name
    };
    ImportedBrush::new(&name, Source::GimpGih, brush, notes)
}

/// ヘッダーの 1 行（`\n` まで、末尾の `\r` は除く）。
fn line(data: &[u8], position: &mut usize) -> Result<String, Fault> {
    let rest = &data[(*position).min(data.len())..];
    let end = rest.iter().position(|&b| b == b'\n');
    match end {
        Some(end) if end <= 4096 => {
            let text = String::from_utf8_lossy(&rest[..end])
                .trim_end_matches('\r')
                .to_string();
            *position += end + 1;
            Ok(text)
        }
        _ => Err(Fault::GihHeader),
    }
}

/// `.vbr`（GIMP のパラメトリックブラシ）。円は丸い筆先に、正方形・ひし形・スパイクは筆先の画像に描く。硬さの落ち方はこのエンジンのもの
/// （GIMP の曲線は文書化されていない）。
pub(crate) fn read_vbr(
    text: &str,
    fallback: Option<&str>,
    budget: &mut Budget,
) -> Result<ImportedBrush, BrushImportError> {
    // 読むのは先頭の VBR_LINES 行だけ。長さは MAX_VBR_BYTES で断ってあるが、行の一覧も全部は持たない。
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.split('\n').take(VBR_LINES).collect();
    let line = |i: usize| -> Result<&str, Fault> {
        lines
            .get(i)
            .map(|l| l.trim())
            .ok_or(Fault::VbrEndsEarly { line: i + 1 })
    };
    let number = |i: usize, min: f64, max: f64| -> Result<f64, Fault> {
        // .NET の数の読み取りは、数のあとの NUL（0 の文字）を読み捨てる。C# の読み手が受け入れる入力は同じに受け入れる。
        match line(i)?.trim_end_matches('\0').parse::<f64>() {
            Ok(v) if !v.is_nan() && v >= min && v <= max => Ok(v),
            _ => Err(Fault::VbrNumber {
                line: i + 1,
                min,
                max,
            }),
        }
    };
    if line(0)? != "GIMP-VBR" {
        return Err(Fault::NotVbr.into());
    }
    let version = line(1)?.to_string();
    let mut name = short_text(line(2)?, 128);
    if name.is_empty() {
        name = fallback.unwrap_or("Untitled").to_string();
    }
    let (mut shape, mut spikes) = (VbrShape::Circle, 2u32);
    let (spacing, radius, hardness, aspect, angle);
    if version == "1.0" {
        spacing = number(3, 0.0, 5000.0)?;
        radius = number(4, 0.1, 4000.0)?;
        hardness = number(5, 0.0, 1.0)?;
        aspect = number(6, 1.0, 20.0)?;
        angle = number(7, 0.0, 180.0)?;
    } else if version == "1.5" {
        shape = match line(3)? {
            "circle" => VbrShape::Circle,
            "square" => VbrShape::Square,
            "diamond" => VbrShape::Diamond,
            other => return Err(Fault::VbrShape(short_text(other, 32)).into()),
        };
        spacing = number(4, 0.0, 5000.0)?;
        radius = number(5, 0.1, 4000.0)?;
        spikes = number(6, 2.0, 20.0)? as u32;
        hardness = number(7, 0.0, 1.0)?;
        aspect = number(8, 1.0, 20.0)?;
        angle = number(9, 0.0, 180.0)?;
    } else {
        return Err(Fault::VbrVersion(short_text(&version, 32)).into());
    }
    let mut brush = Brush {
        base: BrushSettings {
            radius,
            hardness,
            spacing: (spacing / 100.0).clamp(0.01, 4.0),
            ..BrushSettings::default()
        },
        ..Brush::default()
    };
    brush.tip.roundness = (1.0 / aspect).max(0.01);
    brush.tip.angle = angle;
    let mut notes = Vec::new();
    if shape != VbrShape::Circle || spikes > 2 {
        budget.take(SHAPE_SIZE as u64 * SHAPE_SIZE as u64)?;
        brush.tip.image = Some(Arc::new(render_shape(&name, shape, spikes, hardness)));
        notes.push(Unrepresented::VbrShapeRendered { shape, spikes });
    }
    ImportedBrush::new(&name, Source::GimpVbr { version }, brush, notes)
}

/// 1.5 版の最後の欄（角度）が 10 行目。
const VBR_LINES: usize = 10;

const SHAPE_SIZE: usize = 256;

fn render_shape(name: &str, shape: VbrShape, spikes: u32, hardness: f64) -> BrushTip {
    let size = SHAPE_SIZE;
    let mut alpha = vec![0u8; size * size];
    for y in 0..size {
        for x in 0..size {
            let u = (x as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let v = (y as f64 + 0.5) / size as f64 * 2.0 - 1.0;
            let d = if spikes > 2 {
                // 星: 1 つのスパイクの半分の扇へ折りたたみ（0 がスパイクの軸）、角が開くほど縁を内へ引いて、谷を半径の 45% にする
                let half = std::f64::consts::PI / spikes as f64;
                let a = v.atan2(u);
                let r = (u * u + v * v).sqrt();
                let a = ((((a + half) % (2.0 * half)) + 2.0 * half) % (2.0 * half) - half).abs(); // スパイクの軸（最初は +x）で 0
                r * (1.0 + (1.0 / 0.45 - 1.0) * a / half)
            } else {
                match shape {
                    VbrShape::Square => u.abs().max(v.abs()),
                    VbrShape::Diamond => u.abs() + v.abs(),
                    VbrShape::Circle => (u * u + v * v).sqrt(),
                }
            };
            let c = if d > 1.0 {
                0.0
            } else if d <= hardness {
                1.0
            } else {
                smooth((1.0 - d) / (1.0 - hardness))
            };
            alpha[y * size + x] = to_byte(c);
        }
    }
    BrushTip::new(name, size as u32, size as u32, alpha).expect("筆先の大きさは 1〜2048")
}

fn smooth(t: f64) -> f64 {
    t * t * (3.0 - 2.0 * t)
}
