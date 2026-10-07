//! 保存（.ylp）と PSD の速さ。`tools/bench-profiles.py` が yolu-io の example として一時的に置いてビルドする（リポジトリには入れない）。
//!   io_bench [回数]
//! 2048² の 4 レイヤー（なめらかな濃淡に低い桁の雑音。圧縮が実際に働く）で、.ylp の保存（正本の組み立て・合成の PNG・ZIP）と読み込み、
//! PSD の書き出し（RLE・レイヤーを流して書く）と読み込み。出力の形は yolu-core の `bench` と同じ（`label: 最小 x ms / 中央 y ms（n 回）`）。
use std::io::Cursor;
use std::time::Instant;
use yolu_core::{Channel, Document, LayerId, TileCoord};
use yolu_io::psd::{self, Compression, ExportControl, ExportMode, ExportOptions, Limits};
use yolu_io::{composite_pngs, MaterialRef, NativeDocument, Project, SetSpec, WriterInfo};

const SIZE: u32 = 2048;
const SET: &str = "0f1e2d3c-4b5a-4978-8796-a5b4c3d2e1f0";

struct Noise(u64);
impl Noise {
    fn next(&mut self) -> u8 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 32) as u8
    }
}

fn fill(doc: &mut Document, layer: LayerId, seed: u64) {
    let ts = doc.tile_size();
    let mut noise = Noise(seed | 1);
    for ty in 0..SIZE.div_ceil(ts) {
        for tx in 0..SIZE.div_ceil(ts) {
            let mut tile = vec![0u8; (ts * ts * 4) as usize];
            for y in 0..ts {
                for x in 0..ts {
                    let (gx, gy) = (tx * ts + x, ty * ts + y);
                    let base = ((gx * 255 / SIZE) as u8, (gy * 255 / SIZE) as u8, ((gx + gy) * 255 / (2 * SIZE)) as u8);
                    let i = ((y * ts + x) * 4) as usize;
                    tile[i] = base.0.saturating_add(noise.next() & 15);
                    tile[i + 1] = base.1.saturating_add(noise.next() & 15);
                    tile[i + 2] = base.2.saturating_add(noise.next() & 15);
                    tile[i + 3] = 255;
                }
            }
            doc.import_tile(layer, Channel::Color, TileCoord::new(tx, ty), &tile).unwrap();
        }
    }
}

fn measure(label: &str, runs: usize, mut work: impl FnMut()) {
    work(); // 予熱
    let mut ms: Vec<f64> = (0..runs)
        .map(|_| {
            let t = Instant::now();
            work();
            t.elapsed().as_secs_f64() * 1000.0
        })
        .collect();
    ms.sort_by(f64::total_cmp);
    println!("{label}: 最小 {:.2} ms / 中央 {:.2} ms（{runs} 回）", ms[0], ms[runs / 2]);
}

fn main() {
    let runs: usize = std::env::args().nth(1).map_or(5, |s| s.parse().unwrap());
    println!("Rust / io_bench / 論理プロセッサ {}", std::thread::available_parallelism().map_or(0, |n| n.get()));
    let mut doc = Document::new(SIZE, SIZE).unwrap();
    for n in 0..4 {
        let layer = doc.add_layer(&format!("レイヤー {n}")).unwrap();
        fill(&mut doc, layer, 7 + n as u64);
    }
    let writer = WriterInfo { app: "bench".into(), version: "0".into(), unity: "standalone".into() };
    let save = |doc: &Document| -> Vec<u8> {
        let spec = SetSpec {
            id: SET.into(),
            name: "セット".into(),
            material: MaterialRef::Unassigned,
            document: Some(NativeDocument::from_core(doc).unwrap().into()),
            composites: composite_pngs(doc).unwrap(),
        };
        Project::create(writer.clone(), &[spec], SET).unwrap().to_bytes().unwrap()
    };
    let ylp = save(&doc);
    measure("保存 .ylp 2048² 4 レイヤー（正本・合成の PNG・ZIP）", runs, || {
        std::hint::black_box(save(&doc));
    });
    measure("開く .ylp 2048² 4 レイヤー（読み込みと core の文書へ）", runs, || {
        let project = Project::read(&ylp).unwrap();
        std::hint::black_box(project.sets()[0].document.to_core().unwrap());
    });
    let ctl = ExportControl { source_budget: Some(1 << 30), ..ExportControl::default() };
    let options = ExportOptions::new(Channel::Color, ExportMode::Bake);
    let export = |doc: &Document| -> Vec<u8> {
        let plan = psd::plan_export(doc, &options, &ctl).unwrap();
        let mut out = Cursor::new(Vec::new());
        plan.write_psd(doc, &ctl, &mut out, Compression::Rle).unwrap();
        out.into_inner()
    };
    let psd_bytes = export(&doc);
    measure("PSD 書き出し 2048² 4 レイヤー（RLE・流して書く）", runs, || {
        std::hint::black_box(export(&doc));
    });
    let limits = Limits::for_export(1 << 30);
    measure("PSD 読み込み 2048² 4 レイヤー", runs, || {
        let read = psd::read(&psd_bytes, &limits).unwrap();
        assert!(read.document().is_some(), "{:?}", read.diagnostics());
    });
    eprintln!("ylp {} B / psd {} B", ylp.len(), psd_bytes.len());
}
