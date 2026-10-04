//! Unity 版（C#）の読み手との照合。手で組んだ事例と、それを壊した入力（全バイトの書き換え・切り詰め・極端な値）を C# の
//! GimpBrushReader・PhotoshopBrushReader・PhotoshopPatternReader に通した結果（tests/fixtures/brushes/、
//! tools/csharp-golden/brushes.sh で作る）と、Rust の読み手の結果を比べる: 取り込めたか・断ったか・取り込んだ設定の指紋が同じか。
//!
//! 同じにならないと分かっている所（Rust が C# より正確・安全にした所）は、Rust の指紋を C# と同じ形に直して比べる:
//! 筆先の反転・ペンの軸の回転・質感の合わせ方 8 種は core の拡張に写すので、C# が出す「未対応」の注記の数に足す。C# が
//! BrushImportException 以外の例外（範囲外の値の検証）で落ちる入力は、Rust は範囲に収めるか設定を断るので比べない。
//! 設定の記述子に有限でない数（NaN・無限大）がある入力は、C# は使わない項目なら読み進め、使う項目なら検証の例外で落ちる。
//! Rust は設定の節を読めないものとして、筆先だけを注記つきで取り込む（壊れた数の周りの設定を信用しない）ので、比べない。

mod brush_files;

use brush_files::corpus::{self, Case};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::sync::Arc;
use yolu_core::{BrushTip, TextureMode};
use yolu_io::brushes::{import_bytes, Fault, FileKind, ImportedBrush, Unrepresented};

fn bits(v: f64) -> String {
    format!("{:016x}", v.to_bits())
}
fn sha(bytes: &[u8], n: usize) -> String {
    Sha256::digest(bytes)
        .iter()
        .take(n)
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn tip_bytes(t: &BrushTip) -> Vec<u8> {
    let mut v = (t.width() as i32).to_le_bytes().to_vec();
    v.extend((t.height() as i32).to_le_bytes());
    v.extend_from_slice(t.alpha());
    v
}
fn tip_desc(t: Option<&Arc<BrushTip>>) -> String {
    t.map_or("-".into(), |t| {
        format!("{}x{}:{}", t.width(), t.height(), sha(&tip_bytes(t), 8))
    })
}
fn tips_desc(ts: &[Arc<BrushTip>]) -> String {
    if ts.is_empty() {
        return "-".into();
    }
    let all: Vec<u8> = ts.iter().flat_map(|t| tip_bytes(t)).collect();
    format!("{}:{}", ts.len(), sha(&all, 8))
}
fn flag(b: bool) -> &'static str {
    if b {
        "1"
    } else {
        "0"
    }
}

/// 名前の整え方（C# の BrushGolden.Clean と同じ）。
fn clean(name: &str, collapse: bool) -> String {
    let mut t: String = name.chars().filter(|c| !c.is_control()).collect();
    if collapse {
        let mut o = String::new();
        for c in t.chars() {
            if c == '\u{FFFD}' && o.ends_with('\u{FFFD}') {
                continue;
            }
            o.push(c);
        }
        t = o;
    }
    t.trim().chars().take(128).collect()
}
fn esc(s: &str) -> String {
    let h: String = s.encode_utf16().map(|u| format!("{u:04x}")).collect();
    if h.is_empty() {
        "-".into()
    } else {
        h
    }
}

/// C# の BrushGolden.Settings と同じ並びの指紋（名前を除く）。注記の数は C# が数える形に直す: C# はファイル全体の注記（読めなかった節・
/// 使えなかった模様）を全ブラシの警告に複製するので、Rust がセットに 1 回だけ持つ `file_notes` を足して数える。
fn settings(b: &ImportedBrush, file_notes: usize) -> String {
    let br = &b.brush;
    let dual = br.dual.as_ref().map_or("-".to_string(), |d| {
        [
            bits(d.radius),
            bits(d.hardness),
            bits(d.spacing),
            bits(d.angle),
            bits(d.roundness),
            bits(d.scatter),
            d.count.to_string(),
            (d.mode as i32).to_string(),
            tip_desc(d.tip.as_ref()),
        ]
        .join(",")
    });
    let tex = br.texture.as_ref().map_or("-".to_string(), |t| {
        format!(
            "{},td={},ts={}",
            tip_desc(Some(&t.image)),
            bits(t.depth),
            bits(t.scale)
        )
    });
    // C# が注記にする、Rust が core の拡張へ写したもの
    let mapped = (br.tip.flip_x || br.tip.flip_y) as usize
        + br.controls.rotation_angle as usize
        + br.texture
            .as_ref()
            .map_or(0, |t| (t.mode != TextureMode::Multiply) as usize);
    let (j, c, k) = (&br.jitter, &br.color, &br.controls);
    let f = [
        format!("src={}", b.source.label()),
        format!("r={}", bits(br.base.radius)),
        format!("h={}", bits(br.base.hardness)),
        format!("sp={}", bits(br.base.spacing)),
        format!("op={}", bits(br.base.opacity)),
        format!("fl={}", bits(br.base.flow)),
        format!("an={}", bits(br.tip.angle)),
        format!("ro={}", bits(br.tip.roundness)),
        format!("sj={}", bits(j.size)),
        format!("aj={}", bits(j.angle)),
        format!("rj={}", bits(j.roundness)),
        format!("oj={}", bits(j.opacity)),
        format!("fj={}", bits(j.flow)),
        format!("sc={}", bits(j.scatter)),
        format!("ct={}", j.count),
        format!("ps={}", flag(br.base.pressure_size)),
        format!("po={}", flag(br.base.pressure_opacity)),
        format!("pf={}", flag(br.base.pressure_flow)),
        format!("fd={}", flag(br.tip.follow_direction)),
        format!("fb={}", bits(c.foreground_background)),
        format!("hu={}", bits(c.hue)),
        format!("sa={}", bits(c.saturation)),
        format!("br={}", bits(c.brightness)),
        format!("pu={}", bits(c.purity)),
        format!("pt={}", flag(c.per_tip)),
        format!("fs={}", k.fade_size),
        format!("fo={}", k.fade_opacity),
        format!("ff={}", k.fade_flow),
        format!("ti={}", flag(k.tilt_size)),
        format!("to={}", flag(k.tilt_opacity)),
        format!("tf={}", flag(k.tilt_flow)),
        format!("ta={}", flag(k.tilt_angle)),
        format!("tip={}", tip_desc(br.tip.image.as_ref())),
        format!("tips={}", tips_desc(&br.tip.images)),
        format!("sel={}", br.tip.selection as i32),
        format!("tex={tex}"),
        format!("dual={dual}"),
        format!("warn={}", b.unrepresented.len() + file_notes + mapped),
    ];
    f.join(";")
}

struct Outcome {
    ok: bool,
    /// 設定の記述子に有限でない数があり、設定の節を読めないものとして扱った。
    non_finite: bool,
    noname: String,
    names: String,
}

fn run(case_kind: FileKind, bytes: &[u8]) -> Outcome {
    let collapse = matches!(case_kind, FileKind::Gbr | FileKind::Gih | FileKind::Vbr);
    // C# の読み手は代わりの名前に "F" を渡して呼ぶ（PAT は無し）
    match import_bytes(case_kind, bytes, Some("F")) {
        Ok(set) => Outcome {
            ok: true,
            non_finite: set.notes.contains(&Unrepresented::PresetsUnreadable(
                Fault::DescriptorNotFinite,
            )),
            noname: set
                .brushes
                .iter()
                .map(|b| settings(b, set.notes.len()))
                .collect::<Vec<_>>()
                .join("||"),
            names: set
                .brushes
                .iter()
                .map(|b| esc(&clean(&b.name, collapse)))
                .collect::<Vec<_>>()
                .join("||"),
        },
        Err(_) => Outcome {
            ok: false,
            non_finite: false,
            noname: String::new(),
            names: String::new(),
        },
    }
}

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/brushes/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("{path} を読めない（tools/csharp-golden/brushes.sh で作る）: {e}")
    })
}

#[test]
fn hand_built_files_match_the_csharp_readers() {
    let expected = fixture("cases.txt");
    let cases: Vec<Case> = corpus::all().into_iter().filter(|c| c.tag == 1).collect();
    let lines: Vec<&str> = expected.lines().collect();
    assert_eq!(
        lines.len(),
        cases.len(),
        "事例の数が正解と違う。事例を変えたら tools/csharp-golden/brushes.sh で正解を作り直す"
    );
    let mut failures = Vec::new();
    let mut ok_cases = 0;
    for (case, line) in cases.iter().zip(&lines) {
        let parts: Vec<&str> = line.split('\t').collect();
        assert_eq!(parts[0], case.label, "並びが正解と違う");
        let rust = run(case.kind, &case.bytes);
        match parts[1] {
            "EXC" => failures.push(format!(
                "{}: C# が想定外の例外で落ちた（手で組んだ事例では起きないはず）",
                case.label
            )),
            "ERR" => {
                if rust.ok {
                    failures.push(format!("{}: C# は断るが Rust は取り込む", case.label));
                }
            }
            "OK" => {
                ok_cases += 1;
                if !rust.ok {
                    failures.push(format!("{}: C# は取り込むが Rust は断る", case.label));
                    continue;
                }
                let (cs_noname, cs_names) = (parts[2], parts[3]);
                if rust.names != cs_names {
                    failures.push(format!(
                        "{}: 名前が違う\n  C#   {cs_names}\n  Rust {}",
                        case.label, rust.names
                    ));
                }
                if rust.noname != cs_noname {
                    let (r, c): (Vec<&str>, Vec<&str>) = (
                        rust.noname.split("||").collect(),
                        cs_noname.split("||").collect(),
                    );
                    if r.len() != c.len() {
                        failures.push(format!(
                            "{}: ブラシの数が違う（C# {}、Rust {}）",
                            case.label,
                            c.len(),
                            r.len()
                        ));
                    } else {
                        for (i, (rb, cb)) in r.iter().zip(&c).enumerate() {
                            if rb != cb {
                                let diff: Vec<String> = rb
                                    .split(';')
                                    .zip(cb.split(';'))
                                    .filter(|(a, b)| a != b)
                                    .map(|(a, b)| format!("Rust {a} / C# {b}"))
                                    .collect();
                                failures.push(format!(
                                    "{}: {} 番目のブラシが違う: {}",
                                    case.label,
                                    i + 1,
                                    diff.join(", ")
                                ));
                            }
                        }
                    }
                }
            }
            other => panic!("{other}"),
        }
    }
    assert!(
        failures.is_empty(),
        "C# との違い {} 件:\n{}",
        failures.len(),
        failures.join("\n")
    );
    assert!(ok_cases > 90, "取り込める事例が少なすぎる: {ok_cases}");
}

#[test]
fn corrupted_files_are_accepted_and_refused_like_the_csharp_readers() {
    let expected = fixture("fuzz.txt");
    let cases: Vec<Case> = corpus::all().into_iter().filter(|c| c.tag == 0).collect();
    let lines: Vec<&str> = expected.lines().collect();
    assert_eq!(
        lines.len(),
        cases.len(),
        "壊した入力の数が正解と違う。変えたら tools/csharp-golden/brushes.sh で正解を作り直す"
    );
    let (mut compared, mut skipped) = (0usize, 0usize);
    let mut stats: BTreeMap<&str, usize> = BTreeMap::new();
    let mut failures = Vec::new();
    let offset = corpus::hand_built().len() + corpus::bundled_gimp().len();
    for (n, (case, line)) in cases.iter().zip(&lines).enumerate() {
        let (index, p) = (offset + n, line.split(' ').collect::<Vec<&str>>());
        let label = format!("#{index} {}", case.label);
        if p[0] == "X" {
            if std::env::var("BRUSH_GOLDEN_VERBOSE").is_ok() {
                let rust = run(case.kind, &case.bytes);
                eprintln!(
                    "C# が例外: {label} → Rust {}",
                    if rust.ok { "取り込む" } else { "断る" }
                );
            }
            skipped += 1;
            continue;
        }
        let rust = run(case.kind, &case.bytes);
        compared += 1;
        match p[0] {
            "E" => {
                *stats.entry("両方が断る").or_default() += (!rust.ok) as usize;
                if rust.ok {
                    failures.push(format!("{label}: C# は断るが Rust は取り込む"));
                }
            }
            "O" => {
                if !rust.ok {
                    failures.push(format!("{label}: C# は取り込むが Rust は断る"));
                    continue;
                }
                if rust.non_finite {
                    *stats.entry("有限でない数（比べない）").or_default() += 1;
                    continue;
                }
                *stats.entry("両方が取り込む").or_default() += 1;
                if sha(rust.noname.as_bytes(), 3) != p[1] {
                    failures.push(format!("{label}: 設定の指紋が違う\n  Rust {}", rust.noname));
                } else if p[3] == "P" && sha(rust.names.as_bytes(), 3) != p[2] {
                    failures.push(format!("{label}: 名前が違う（Rust {}）", rust.names));
                }
            }
            other => panic!("{other}"),
        }
    }
    eprintln!(
        "比べた {compared} 件（C# が想定外の例外で落ちて比べなかった {skipped} 件）: {stats:?}"
    );
    assert!(
        failures.is_empty(),
        "C# との違い {} 件（初めの 15 件）:\n{}",
        failures.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        stats.get("両方が取り込む").copied().unwrap_or(0) > 1000
            && stats.get("両方が断る").copied().unwrap_or(0) > 1000
    );
    // 数を固定する（文書・コミットメッセージが引く数）。壊した入力は全部で 11,522 件で、そのうち C# が想定外の例外で落ちる 14 件は
    // 比べず、残りの 11,508 件を比べる。比べた中で、有限でない数のある設定の節を持つ 3 件は設定の指紋を比べない。
    // 変えるときは、理由を確かめてから直す。
    assert_eq!(cases.len(), 11_522);
    assert_eq!((compared, skipped), (11_508, 14));
    assert_eq!(stats.get("有限でない数（比べない）").copied(), Some(3));
}

/// C# の正解を作るための入力（tools/csharp-golden/brushes.sh が呼ぶ）。
#[test]
#[ignore]
fn export_corpus() {
    let path = std::env::var("BRUSH_CORPUS_OUT").expect("BRUSH_CORPUS_OUT");
    let mut out = b"YBRI\x01".to_vec();
    for c in corpus::all() {
        let label = if c.tag == 1 { c.label.as_bytes() } else { b"" };
        out.push(c.tag);
        out.push(corpus::kind_code(c.kind));
        out.push(label.len() as u8);
        out.extend_from_slice(label);
        out.extend((c.bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&c.bytes);
    }
    std::fs::write(path, out).unwrap();
}
