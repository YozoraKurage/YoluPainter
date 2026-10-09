//! 他の画像編集アプリと同じ文書・同じ操作で、core・io を直に呼んだ時間を測る台（CPU だけ。アプリの画面の時間とは別）。
//!
//! 使い方:
//! - 合成の文書を PSD に書き出す: `appbench gen <sparse|dense> <辺> <レイヤーの数> <出力.psd>`
//! - 文書の構成を出す（レイヤーの名前は出さない）: `appbench info <入力.psd>`
//! - 測る: `appbench run <入力.psd> <ラベル> [--repeat 3] [--out-dir <作業のフォルダ>] [--dump <PNG の出力先>] [--only 操作,操作]`
//!
//! 出力は `ラベル<TAB>操作<TAB>中央値(ms)<TAB>各回(ms)` の TSV（標準出力）と、注記（標準エラー）。
//! 操作（`OPERATIONS`）: open（どの操作にも要るので `--only` に関係なく測る）・composite・composite_warm・toggle・opacity・toggle2・
//! opacity2（一番上の見た目に効くレイヤー）・blur_r8・blur_r40・levels・filter_doc_blur8（フィルターをレイヤーに付けて全体を合成）・png・
//! psd_plan・psd_write・psd_verify（PSD の保存の計画・書き出し・書いた直後の確かめ。確かめはアプリの保存と同じ `psd::check_written`
//! で、読み戻しと書いたバイト列との照合）・merge（許容差 255 は差があっても断らない設定）・
//! merge_app（アプリの既定の許容差）。`--only` は名前ごとに選べて、選んだ操作の行だけ出す（知らない名前は断る）。
//! 合成の文書は他の画像編集アプリでも開けるよう、レイヤーは Normal/Multiply/Screen/Overlay のラスターだけで作る。

use std::io::{BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use yolu_app::engine::{Channel, Document, LayerId, LayerKind, Rgba8};
use yolu_core::filter::{self, Image, Options, Settings, Stage, ValueType};
use yolu_core::glam::DVec2;
use yolu_core::{BlendMode, BrushSettings, Rect};
use yolu_io::psd::{self, Compression, ExportControl, ExportMode, ExportOptions};

/// 乱数（毎回同じ文書になる）。
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// 測れる操作の名前（`--only` に渡せる名前）。
const OPERATIONS: &[&str] = &[
    "open",
    "composite",
    "composite_warm",
    "toggle",
    "toggle2",
    "opacity",
    "opacity2",
    "blur_r8",
    "blur_r40",
    "levels",
    "filter_doc_blur8",
    "png",
    "psd_plan",
    "psd_write",
    "psd_verify",
    "merge",
    "merge_app",
];

fn median(v: &[f64]) -> f64 {
    let mut v = v.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v.get(v.len() / 2).copied().unwrap_or(0.0)
}

const MODES: [BlendMode; 6] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Normal,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::Normal,
];

fn brush(rng: &mut Rng, radius: f64, opacity: f64) -> BrushSettings {
    let r = rng.next();
    BrushSettings {
        radius,
        hardness: 0.9,
        spacing: 0.3,
        opacity,
        flow: 1.0,
        color: Rgba8::new(r as u8, (r >> 8) as u8, (r >> 16) as u8, 255),
        pressure_size: false,
        pressure_opacity: false,
        pressure_flow: false,
        erase: false,
        anti_alias: yolu_core::AntiAlias::None,
    }
}

/// 部分的な文書: 下地（全面を丸い大きな筆で埋める）と、レイヤーごとに丸い筆の跡を何本か置いたレイヤー（絵に近い疎な形）。
fn gen_sparse(size: u32, layers: usize) -> Document {
    let mut d = Document::with_tile_size(size, size, 128).unwrap();
    d.set_source_budget_bytes(4 << 30).unwrap();
    let mut rng = Rng(0xC0FFEE);
    let bg = d.add_layer("bg").unwrap();
    // 下地は全面を不透明に
    let b = brush(&mut rng, size as f64 / 6.0, 1.0);
    let mut s = d.begin_stroke(bg, &b).unwrap();
    let step = size as f64 / 8.0;
    let mut y = 0.0;
    while y < size as f64 + step {
        let mut x = 0.0;
        while x < size as f64 + step {
            s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
            x += step;
        }
        y += step;
    }
    d.end_stroke(s).unwrap();
    for i in 1..layers {
        let id = d.add_layer(&format!("L{i}")).unwrap();
        for _ in 0..(6 + rng.below(10)) {
            let radius = size as f64 / 24.0 + rng.below(size as u64 / 8) as f64;
            let opacity = 0.5 + rng.below(50) as f64 / 100.0;
            let b = brush(&mut rng, radius, opacity);
            let mut s = d.begin_stroke(id, &b).unwrap();
            let (x, y) = (rng.below(size as u64) as f64, rng.below(size as u64) as f64);
            s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
            s.add_point(&mut d, x + 40.0, y + 12.0, 1.0, DVec2::ZERO)
                .unwrap();
            d.end_stroke(s).unwrap();
        }
        d.set_layer_blend_mode(id, MODES[i % MODES.len()]).unwrap();
        d.set_layer_opacity(id, 0.6 + 0.04 * (i % 10) as f64, false)
            .unwrap();
    }
    d.clear_history().unwrap();
    d
}

/// 全面の文書: 全レイヤーがキャンバスいっぱいに半透明の画素を持つ（合成が一番重い形）。
fn gen_dense(size: u32, layers: usize) -> Document {
    let mut d = Document::with_tile_size(size, size, 128).unwrap();
    d.set_source_budget_bytes(4 << 30).unwrap();
    let mut rng = Rng(0xD3E5E);
    for i in 0..layers {
        let id = d.add_layer(&format!("D{i}")).unwrap();
        let mut b = brush(&mut rng, 48.0, if i == 0 { 1.0 } else { 0.35 });
        b.hardness = 1.0;
        b.spacing = 0.2;
        let mut s = d.begin_stroke(id, &b).unwrap();
        let step = 40.0;
        let mut y = 0.0;
        while y < size as f64 + step {
            let mut x = 0.0;
            while x < size as f64 + step {
                s.add_point(&mut d, x, y, 1.0, DVec2::ZERO).unwrap();
                x += step;
            }
            y += step;
        }
        d.end_stroke(s).unwrap();
        d.set_layer_blend_mode(id, MODES[i % MODES.len()]).unwrap();
        d.set_layer_opacity(id, 0.5 + 0.04 * (i % 10) as f64, false)
            .unwrap();
    }
    d.clear_history().unwrap();
    d
}

fn write_psd(doc: &Document, path: &Path) -> Result<(u64, f64, f64, psd::Written), String> {
    // アプリと同じく、レイヤーのメモリの予算を渡す（渡さないと、書くファイルの上限が固定の 128 MiB になる）
    let ctl = ExportControl {
        source_budget: Some(4 << 30),
        ..ExportControl::default()
    };
    let t = Instant::now();
    let plan = psd::plan_export(
        doc,
        &ExportOptions::new(Channel::Color, ExportMode::Bake),
        &ctl,
    )
    .map_err(|e| format!("{e:?}"))?;
    let plan_ms = ms(t);
    let t = Instant::now();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let written = plan
        .write_psd(doc, &ctl, &mut out, Compression::Rle)
        .map_err(|e| format!("{e:?}"))?;
    out.flush().map_err(|e| e.to_string())?;
    let write_ms = ms(t);
    let bytes = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    Ok((bytes, plan_ms, write_ms, written))
}

fn open_psd(path: &Path) -> Document {
    let file = std::fs::File::open(path).expect("PSD を開けない");
    // アプリの取り込み（`psd::import_worker`）と同じ緩衝
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let options = psd::CopyOptions {
        source_budget: 8 * 1024 * 1024 * 1024,
        cancel: None,
    };
    match psd::import_copy(&mut reader, &options).expect("取り込みに失敗") {
        psd::CopyOutcome::Imported(i) => i.document,
        psd::CopyOutcome::Refused(r) => panic!("取り込めない: {r:?}"),
    }
}

/// 合成に出るか（自分も親のグループも、表示していて不透明度が 0 でない）。
fn shown(doc: &Document, l: &yolu_app::engine::Layer) -> bool {
    let mut cur = Some(l);
    while let Some(layer) = cur {
        if !layer.visible() || layer.opacity() <= 0.0 {
            return false;
        }
        cur = layer.parent().and_then(|p| doc.layer(p));
    }
    true
}

/// レイヤーの色の面のタイルの外接矩形（画素の座標）。
fn tile_bbox(doc: &Document, id: LayerId) -> Rect {
    let Some(surface) = doc.layer(id).and_then(|l| l.surface(Channel::Color)) else {
        return Rect::new(0, 0, 0, 0);
    };
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    for c in surface.tile_coords() {
        if let Some(r) = doc.tile_rect(c) {
            x0 = x0.min(r.x);
            y0 = y0.min(r.y);
            x1 = x1.max(r.x + r.width);
            y1 = y1.max(r.y + r.height);
        }
    }
    if x0 == u32::MAX {
        return Rect::new(0, 0, 0, 0);
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// 描けるレイヤー（ラスター）の、下から数えた葉の番号（グループは数えない）。他のアプリの台本へ渡して、同じレイヤーを選ぶ。
fn leaf_order(doc: &Document) -> Vec<LayerId> {
    doc.layers()
        .iter()
        .filter(|l| l.kind() != LayerKind::Group)
        .map(|l| l.id())
        .collect()
}

/// 操作の対象のレイヤー: 合成に出るラスターのうち、タイルが一番多いレイヤー（同数なら下のレイヤー）。
fn target_layer(doc: &Document) -> LayerId {
    let mut best: Option<(usize, LayerId)> = None;
    for l in doc.layers() {
        if l.kind() != LayerKind::Raster || !shown(doc, l) {
            continue;
        }
        let n = l.surface(Channel::Color).map_or(0, |s| s.tile_count());
        if best.is_none_or(|(m, _)| n > m) {
            best = Some((n, l.id()));
        }
    }
    best.expect("ラスターレイヤーが無い").1
}

/// 2 つ目の対象のレイヤー: 合成に出るラスターのうち、タイルが一番多いレイヤーの 3 分の 1 以上持つレイヤーで、一番上のもの（見た目に効くレイヤー。1 つ目は一番下のレイヤーに
/// なりがちで、上の不透明なレイヤーに隠れていると切り替えても合成が変わらない）。
fn target2_layer(doc: &Document) -> LayerId {
    let max = doc
        .layers()
        .iter()
        .filter(|l| l.kind() == LayerKind::Raster && shown(doc, l))
        .map(|l| l.surface(Channel::Color).map_or(0, |s| s.tile_count()))
        .max()
        .unwrap_or(0);
    doc.layers()
        .iter()
        .rev()
        .find(|l| {
            l.kind() == LayerKind::Raster
                && shown(doc, l)
                && l.surface(Channel::Color).map_or(0, |s| s.tile_count()) >= (max / 3).max(1)
        })
        .map(|l| l.id())
        .expect("ラスターレイヤーが無い")
}

fn info(path: &Path) {
    let doc = open_psd(path);
    let leaves = leaf_order(&doc);
    let target = target_layer(&doc);
    let target2 = target2_layer(&doc);
    println!(
        "# {}×{} レイヤー {}（葉 {}）対象の葉の番号(下から 0 始まり)={} 2 つ目の対象={}",
        doc.width(),
        doc.height(),
        doc.layers().len(),
        leaves.len(),
        leaves.iter().position(|i| *i == target).unwrap(),
        leaves.iter().position(|i| *i == target2).unwrap()
    );
    println!("葉\t種類\t表示\t不透明度\t合成\tクリップ\tマスク\t親\tタイル\t外接(x,y,w,h)");
    for (n, id) in leaves.iter().enumerate() {
        let l = doc.layer(*id).unwrap();
        let bb = tile_bbox(&doc, *id);
        println!(
            "{n}\t{:?}\t{}\t{:.3}\t{:?}\t{}\t{}\t{}\t{}\t{},{},{},{}",
            l.kind(),
            l.visible(),
            l.opacity(),
            l.blend_mode(),
            l.clipping(),
            l.mask().is_some(),
            l.parent().is_some(),
            l.surface(Channel::Color).map_or(0, |s| s.tile_count()),
            bb.x,
            bb.y,
            bb.width,
            bb.height
        );
    }
    let groups = doc
        .layers()
        .iter()
        .filter(|l| l.kind() == LayerKind::Group)
        .count();
    println!("# グループ {groups}");
}

struct Recorder {
    label: String,
    rows: Vec<(String, Vec<f64>)>,
}
impl Recorder {
    fn push(&mut self, op: &str, ms: f64) {
        match self.rows.iter_mut().find(|(o, _)| o == op) {
            Some((_, v)) => v.push(ms),
            None => self.rows.push((op.to_string(), vec![ms])),
        }
    }
    fn print(&self) {
        for (op, v) in &self.rows {
            let list: Vec<String> = v.iter().map(|x| format!("{x:.1}")).collect();
            println!(
                "{}\t{}\t{:.1}\t{}",
                self.label,
                op,
                median(v),
                list.join(" ")
            );
        }
    }
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// レイヤーの画素（straight RGBA8）を、外接矩形だけ取り出す。
fn crop(canvas: &[u8], width: u32, r: Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity(r.width as usize * r.height as usize * 4);
    for y in r.y..r.y + r.height {
        let s = (y as usize * width as usize + r.x as usize) * 4;
        out.extend_from_slice(&canvas[s..s + r.width as usize * 4]);
    }
    out
}

fn run(
    path: &Path,
    label: &str,
    repeat: usize,
    out_dir: &Path,
    dump: Option<&Path>,
    only: Option<&str>,
) -> Recorder {
    let want = |name: &str| only.is_none_or(|o| o.split(',').any(|x| x == name));
    let mut rec = Recorder {
        label: label.to_string(),
        rows: Vec::new(),
    };
    std::fs::create_dir_all(out_dir).unwrap();
    let psd_path = out_dir.join("yolu_out.psd");
    let png_path = out_dir.join("yolu_out.png");
    for rep in 0..repeat {
        let t = Instant::now();
        let mut doc = open_psd(path);
        rec.push("open", ms(t));
        let (w, h) = (doc.width(), doc.height());
        let target = target_layer(&doc);
        let bbox = tile_bbox(&doc, target);
        if rep == 0 {
            let leaves = leaf_order(&doc);
            eprintln!(
                "# {label}: {w}×{h} レイヤー {} 対象の葉={} 対象の外接={}×{}（{},{}）スレッド={}",
                doc.layers().len(),
                leaves.iter().position(|i| *i == target).unwrap(),
                bbox.width,
                bbox.height,
                bbox.x,
                bbox.y,
                rayon::current_num_threads()
            );
        }
        let bounds = doc.bounds();
        // 全体の合成（開いた直後の 1 回目）。2 回目（composite_warm）だけ選んでも、1 回目は先に走らせる（時間は composite を選んだときだけ残す）
        if want("composite") || want("composite_warm") {
            let t = Instant::now();
            let first = doc.composite_channel(Channel::Color, bounds).unwrap();
            if want("composite") {
                rec.push("composite", ms(t));
            }
            if rep == 0 {
                eprintln!("# 合成の指紋 {:016x}", fnv(&first));
            }
            drop(first);
        }
        if rep == 0 {
            if let Some(dir) = dump {
                std::fs::create_dir_all(dir).unwrap();
                std::fs::write(
                    dir.join(format!("{label}.png")),
                    yolu_io::composite_png(&doc).unwrap(),
                )
                .unwrap();
            }
        }
        // 変えずにもう一度（覚えのない道なので、同じ費用になるはず）
        if want("composite_warm") {
            let t = Instant::now();
            let again = doc.composite_channel(Channel::Color, bounds).unwrap();
            rec.push("composite_warm", ms(t));
            drop(again);
        }
        // 表示の切り替え → 合成
        if want("toggle") {
            let t = Instant::now();
            doc.set_layer_visible(target, false).unwrap();
            let c = doc.composite_channel(Channel::Color, bounds).unwrap();
            rec.push("toggle", ms(t));
            drop(c);
            doc.set_layer_visible(target, true).unwrap();
        }
        // 2 つ目の対象（一番上の見た目に効くレイヤー）での切り替えと不透明度
        let target2 = target2_layer(&doc);
        if want("toggle2") {
            let t = Instant::now();
            doc.set_layer_visible(target2, false).unwrap();
            let c = doc.composite_channel(Channel::Color, bounds).unwrap();
            rec.push("toggle2", ms(t));
            drop(c);
            doc.set_layer_visible(target2, true).unwrap();
        }
        if want("opacity2") {
            let t = Instant::now();
            doc.set_layer_opacity(target2, 0.4, false).unwrap();
            let c = doc.composite_channel(Channel::Color, bounds).unwrap();
            rec.push("opacity2", ms(t));
            drop(c);
            doc.set_layer_opacity(target2, 1.0, false).unwrap();
        }
        // 不透明度の変更 → 合成
        if want("opacity") {
            let t = Instant::now();
            doc.set_layer_opacity(target, 0.4, false).unwrap();
            let c = doc.composite_channel(Channel::Color, bounds).unwrap();
            rec.push("opacity", ms(t));
            drop(c);
            doc.set_layer_opacity(target, 1.0, false).unwrap();
        }
        // レイヤーへの破壊的な処理（ぼかし・レベル補正）に当たる、フィルターの計算。対象のレイヤーの外接矩形の画素へ
        let layer_bytes = doc
            .layer(target)
            .and_then(|l| l.surface(Channel::Color))
            .map(|s| s.to_canvas_bytes())
            .unwrap();
        let region = crop(&layer_bytes, w, bbox);
        drop(layer_bytes);
        let image = Image::new(&region, bbox.width, bbox.height).unwrap();
        let whole = Rect::new(0, 0, bbox.width, bbox.height);
        let options = Options {
            working_budget: 8 << 30,
            ..Options::default()
        };
        let stages: [(&str, Settings); 3] = [
            ("blur_r8", Settings::GaussianBlur { radius: 8 }),
            ("blur_r40", Settings::GaussianBlur { radius: 40 }),
            (
                "levels",
                Settings::Levels {
                    input_black: 0.05,
                    input_white: 0.95,
                    gamma: 1.1,
                    output_black: 0.0,
                    output_white: 1.0,
                },
            ),
        ];
        for (name, settings) in &stages {
            if !want(name) {
                continue;
            }
            let t = Instant::now();
            let out = filter::evaluate(
                &image,
                ValueType::Color,
                &[Stage::new(settings.clone())],
                whole,
                &options,
            )
            .unwrap();
            rec.push(name, ms(t));
            std::hint::black_box(&out);
        }
        drop(region);
        // 非破壊の道: フィルターを付けた文書の全体の合成（フィルターの評価を含む）
        if want("filter_doc_blur8") {
            let id = doc
                .add_filter(
                    target,
                    yolu_core::FilterTarget::Content,
                    yolu_core::effects::FilterSpec::new(yolu_core::effects::EffectSettings::blur(
                        8,
                    )),
                )
                .unwrap();
            let t = Instant::now();
            let c = doc.composite_channel(Channel::Color, bounds).unwrap();
            rec.push("filter_doc_blur8", ms(t));
            drop(c);
            doc.remove_filter(target, id).unwrap();
        }
        // PNG の書き出し（合成してから符号化してファイルへ）
        if want("png") {
            let t = Instant::now();
            let bytes = yolu_io::composite_png(&doc).unwrap();
            std::fs::write(&png_path, &bytes).unwrap();
            rec.push("png", ms(t));
            if rep == 0 {
                eprintln!("# PNG {} バイト", bytes.len());
            }
        }
        // PSD の保存（計画・書き出し・書いた直後の読み戻しの確かめ）。3 つは 1 回の保存から取るので、どれかを選べば保存は走り、行は選んだ名前のぶんだけ残す
        let (want_plan, want_write, want_verify) =
            (want("psd_plan"), want("psd_write"), want("psd_verify"));
        if want_plan || want_write || want_verify {
            match write_psd(&doc, &psd_path) {
                Ok((bytes, plan_ms, write_ms, written)) => {
                    if want_plan {
                        rec.push("psd_plan", plan_ms);
                    }
                    if want_write {
                        rec.push("psd_write", write_ms);
                    }
                    if rep == 0 {
                        eprintln!("# PSD {bytes} バイト");
                    }
                    if want_verify {
                        let t = Instant::now();
                        let mut file = std::fs::File::open(&psd_path).unwrap();
                        psd::check_written(&psd_path, &mut file, written, None).unwrap();
                        rec.push("psd_verify", ms(t));
                    }
                }
                Err(e) => eprintln!("# PSD の保存は断られた: {e}"),
            }
        }
        // 全レイヤーの結合（表示に寄与するレイヤーを 1 枚へ）。許容差 255 は、結果と元の合成に差があっても断らない設定（比べはどの許容差でも全部のタイルで走る。
        // 許容差は比べたあとの断る判定にだけ効く）。アプリの既定（`MERGE_ROUNDING_TOLERANCE`）の結合は、255 の結合で変わった文書でなく、開き直した文書で測る
        if want("merge") {
            let t = Instant::now();
            match doc.merge_visible("merged", 255) {
                Ok(_) => rec.push("merge", ms(t)),
                Err(e) => eprintln!("# 結合は断られた: {e:?}"),
            }
        }
        if want("merge_app") {
            drop(doc);
            let mut doc = open_psd(path);
            let t = Instant::now();
            match doc.merge_visible("merged", Document::MERGE_ROUNDING_TOLERANCE) {
                Ok(_) => rec.push("merge_app", ms(t)),
                Err(e) => eprintln!("# 既定の許容差の結合は断られた: {e:?}"),
            }
        }
    }
    let _ = std::fs::remove_file(&psd_path);
    let _ = std::fs::remove_file(&png_path);
    rec
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    match args.first().map(String::as_str) {
        Some("gen") => {
            let kind = args.get(1).expect("sparse か dense");
            let size: u32 = args.get(2).and_then(|s| s.parse().ok()).expect("辺");
            let layers: usize = args
                .get(3)
                .and_then(|s| s.parse().ok())
                .expect("レイヤーの数");
            let out = PathBuf::from(args.get(4).expect("出力"));
            let doc = match kind.as_str() {
                "sparse" => gen_sparse(size, layers),
                "dense" => gen_dense(size, layers),
                _ => panic!("sparse か dense"),
            };
            let (bytes, _, _, _) = write_psd(&doc, &out).expect("PSD を書けない");
            eprintln!("{} {bytes} バイト", out.display());
        }
        Some("info") => info(Path::new(args.get(1).expect("入力"))),
        Some("run") => {
            let path = PathBuf::from(args.get(1).expect("入力"));
            let label = args.get(2).expect("ラベル").clone();
            let repeat = arg("--repeat").and_then(|s| s.parse().ok()).unwrap_or(3);
            let out_dir = PathBuf::from(arg("--out-dir").unwrap_or_else(|| "/tmp/appbench".into()));
            let dump = arg("--dump").map(PathBuf::from);
            let only = arg("--only");
            if let Some(unknown) = only.as_deref().and_then(|o| {
                o.split(',')
                    .find(|name| !OPERATIONS.contains(name))
                    .map(str::to_string)
            }) {
                eprintln!(
                    "--only に知らない操作の名前: {unknown}（選べる名前: {}）",
                    OPERATIONS.join("・")
                );
                std::process::exit(2);
            }
            run(
                &path,
                &label,
                repeat,
                &out_dir,
                dump.as_deref(),
                only.as_deref(),
            )
            .print();
        }
        _ => eprintln!("使い方: appbench gen|info|run …（先頭のコメントを参照）"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("appbench-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// 合成の文書は、PSD に書いて読み戻しても合成がほぼ変わらない（他のアプリへ渡す文書が、こちらの文書と同じ絵であること）。
    /// PSD は不透明度を 8 ビットで持つので、0.64 のような値が 163/255 になる分だけ、合成に 1〜2 の差が出る。
    #[test]
    fn generated_documents_survive_the_psd_round_trip() {
        let dir = temp_dir("roundtrip");
        for (name, doc) in [("sparse", gen_sparse(128, 6)), ("dense", gen_dense(128, 4))] {
            let path = dir.join(format!("{name}.psd"));
            let (bytes, _, _, _) = write_psd(&doc, &path).unwrap();
            assert!(bytes > 0);
            let back = open_psd(&path);
            assert_eq!(
                back.layers().len(),
                doc.layers().len(),
                "{name}: レイヤーの数"
            );
            let a = doc.composite_channel(Channel::Color, doc.bounds()).unwrap();
            let b = back
                .composite_channel(Channel::Color, back.bounds())
                .unwrap();
            let max = a.iter().zip(&b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            assert!(max <= 3, "{name}: 合成の最大の差 {max}");
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 全部の操作が小さな文書で通り、操作の名前ごとに回数分の時間が残る。`--only` で場面を絞れる。
    #[test]
    fn run_records_every_operation_once_per_repeat() {
        let dir = temp_dir("run");
        let psd = dir.join("doc.psd");
        write_psd(&gen_sparse(128, 5), &psd).unwrap();
        let rec = run(
            &psd,
            "T",
            2,
            &dir.join("out"),
            Some(&dir.join("dump")),
            None,
        );
        for &op in OPERATIONS {
            let (_, v) = rec
                .rows
                .iter()
                .find(|(o, _)| o == op)
                .unwrap_or_else(|| panic!("{op} が無い"));
            assert_eq!(v.len(), 2, "{op}: 回数");
            assert!(v.iter().all(|t| t.is_finite() && *t >= 0.0), "{op}: 時間");
        }
        assert!(dir.join("dump").join("T.png").exists(), "合成の PNG を出す");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `--only` は操作ごとに選べる。選んだ名前の行だけが残る（open はどの操作にも要るので常に残る）。
    /// psd_plan・psd_verify・merge_app・composite_warm のように、ほかの操作と同じ流れの中で測る操作も、自分の名前で選べる。
    #[test]
    fn only_selects_each_operation_by_its_own_name() {
        let dir = temp_dir("only");
        let psd = dir.join("doc.psd");
        write_psd(&gen_sparse(128, 5), &psd).unwrap();
        for &op in OPERATIONS {
            let rec = run(&psd, "T", 1, &dir.join("out"), None, Some(op));
            let mut names: Vec<&str> = rec.rows.iter().map(|(o, _)| o.as_str()).collect();
            names.sort_unstable();
            let mut expected = vec!["open", op];
            expected.sort_unstable();
            expected.dedup();
            assert_eq!(names, expected, "--only {op}");
        }
        let two = run(
            &psd,
            "T",
            1,
            &dir.join("out"),
            None,
            Some("levels,merge_app"),
        );
        let names: Vec<&str> = two.rows.iter().map(|(o, _)| o.as_str()).collect();
        assert_eq!(names, ["open", "levels", "merge_app"], "複数の名前");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 対象のレイヤーは、表示されているラスターの中でタイルが一番多いレイヤー。2 つ目は一番上の見た目に効くレイヤー。
    #[test]
    fn target_layers_are_chosen_from_shown_rasters() {
        let doc = gen_sparse(128, 6);
        let leaves = leaf_order(&doc);
        assert_eq!(
            leaves.iter().position(|i| *i == target_layer(&doc)),
            Some(0),
            "下地が一番タイルを持つ"
        );
        assert_eq!(
            leaves.iter().position(|i| *i == target2_layer(&doc)),
            Some(leaves.len() - 1)
        );
    }
}
