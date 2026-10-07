//! 人工 RGBA8 2 レイヤーの変形・縮小・結合と、疎なレイヤー（8192²・描いたタイルが少ない）のサイズ変更。準備は計測外、各操作 3 回の中央値。
use std::time::Instant;
use yolu_core::*;
fn document(side: u32) -> (Document, LayerId) {
    let mut d = Document::with_tile_size(side, side, 128).unwrap();
    let mut last = LayerId(0);
    for l in 0..2 {
        last = d.add_layer("人工レイヤー").unwrap();
        for ty in 0..side / 128 {
            for tx in 0..side / 128 {
                let mut bytes = vec![0; 128 * 128 * 4];
                for y in 0..128 {
                    for x in 0..128 {
                        let n = (tx * 128 + x) * 17 + (ty * 128 + y) * 31 + l * 7;
                        let o = ((y * 128 + x) * 4) as usize;
                        bytes[o..o + 4].copy_from_slice(&[
                            (n * 3 + 1) as u8,
                            (n * 5 + 2) as u8,
                            (n * 7 + 3) as u8,
                            if n.is_multiple_of(5) { 0 } else { 128 },
                        ]);
                    }
                }
                d.import_tile(last, Channel::Color, TileCoord::new(tx, ty), &bytes)
                    .unwrap();
            }
        }
    }
    d.clear_history().unwrap();
    (d, last)
}
/// 8192² の 1 レイヤー。`painted` 枚のタイルだけ人工の画素を持ち、`uniform` なら残りの全タイルは一様な色で埋める（無い・一様・
/// 画素ありの 3 種類が混ざる）。
fn sparse_document(painted: u32, uniform: bool) -> Document {
    let side = 8192;
    let mut d = Document::with_tile_size(side, side, 128).unwrap();
    let id = d.add_layer("疎なレイヤー").unwrap();
    let columns = side / 128;
    for i in 0..columns * columns {
        let coord = TileCoord::new(i % columns, i / columns);
        let pixel = |k: u32| -> [u8; 4] {
            if i < painted {
                let n = k * 7 + i;
                [(n * 3 + 1) as u8, (n * 5 + 2) as u8, (n * 7 + 3) as u8, 200]
            } else {
                [40, 90, 160, 255]
            }
        };
        if i >= painted && !uniform {
            continue;
        }
        let bytes: Vec<u8> = (0..128 * 128).flat_map(pixel).collect();
        d.import_tile(id, Channel::Color, coord, &bytes).unwrap();
    }
    d.clear_history().unwrap();
    d
}
fn sparse() {
    for (name, painted, uniform) in [
        ("1 タイルだけ", 1, false),
        ("1 タイル + 残りは一様", 1, true),
    ] {
        for op in ["面積縮小", "双線形拡大", "キャンバスの切り出し"] {
            let mut times = Vec::new();
            for _ in 0..3 {
                let mut d = sparse_document(painted, uniform);
                let start = Instant::now();
                match op {
                    "面積縮小" => {
                        d.resize_image(4096, 4096, CanvasResampling::Area).unwrap();
                    }
                    "双線形拡大" => {
                        d.resize_image(8192, 8000, CanvasResampling::Bilinear)
                            .unwrap();
                    }
                    _ => {
                        d.resize_canvas(4096, 4096, (-1000, -1000)).unwrap();
                    }
                }
                times.push(start.elapsed().as_secs_f64() * 1000.);
                std::hint::black_box(&d);
            }
            times.sort_by(f64::total_cmp);
            println!("疎な 8192² ({name}) {op} 中央値={:.3} ms", times[1]);
        }
    }
}
fn main() {
    if std::env::args().any(|a| a == "--sparse") {
        for degree in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(degree)
                .build()
                .unwrap();
            println!("並列度={degree}");
            pool.install(sparse);
        }
        return;
    }
    for degree in [1, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(degree)
            .build()
            .unwrap();
        pool.install(||{
        for side in [512,1024]{for op in ["変形","面積縮小","下へ結合"]{
            let mut times=Vec::new();let mut bytes=0;let mut history=0;
            for _ in 0..3{let (mut d,id)=document(side);let start=Instant::now();match op{
                "変形"=>{d.transform_layer(id,Affine2D::from_parts((side as f64/2.,side as f64/2.),(1.,-1.),23.,(1.3,0.7)).unwrap(),Resampling::Bilinear,true).unwrap();},
                "面積縮小"=>{d.resize_image(side/2,side/2,CanvasResampling::Area).unwrap();},
                _=>{d.merge_down(id,255).unwrap();}
            }times.push(start.elapsed().as_secs_f64()*1000.);bytes=d.allocated_bytes();history=d.history_bytes();std::hint::black_box(&d);}
            times.sort_by(f64::total_cmp);println!("{side}² {op} 並列度={degree} 中央値={:.3} ms 画素={bytes} B 履歴={history} B",times[1]);
        }}
    });
    }
}
