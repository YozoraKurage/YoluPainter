//! 文字の塗りが、計算の道（SIMD）とスレッドの数によらず同じバイトになること。道を固定する口（`math::simd::forced`）が crate の中に
//! しか無いので、ここに置く（値ごとの正解との照合は `tests/reference/text.rs`）。

use super::*;
use crate::math::simd::forced;

/// 同梱のフォント（アプリの画面のフォントと同じファイル）。
const REGULAR: &[u8] = include_bytes!("../../../yolu-app/assets/fonts/BIZUDPGothic-Regular.ttf");

fn settings() -> TextSettings {
    let mut s = TextSettings::new(
        "あいうえお 漢字とEnglish、混ぜた文。\n二行目は右に揃える Wrap me please",
        TextFont::Bundled("biz-udpgothic-regular".into()),
        20.5,
        230.25,
    );
    s.size = 23.5;
    s.color = Rgba8::new(200, 30, 60, 220);
    s.rotation = 17.0;
    s.letter_spacing = 0.05;
    s.wrap_width = 260.0;
    s.align = TextAlign::Center;
    s
}

fn paint() -> Vec<u8> {
    render(&settings(), REGULAR, 0, 300, 280, 64, u64::MAX)
        .unwrap()
        .to_canvas_bytes()
}

#[test]
fn every_simd_path_and_thread_count_paints_the_same_bytes() {
    let reference = paint();
    assert!(
        reference.chunks_exact(4).filter(|p| p[3] > 0).count() > 2000,
        "文字が塗られていない"
    );
    for level in forced::supported() {
        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let bytes = forced::with_level(level, || pool.install(paint));
            assert!(
                bytes == reference,
                "道 {level:?}・スレッド {threads} で結果が違う"
            );
        }
    }
}

#[test]
fn the_tile_size_does_not_change_the_pixels() {
    let s = settings();
    let a = render(&s, REGULAR, 0, 300, 280, 8, u64::MAX).unwrap();
    let b = render(&s, REGULAR, 0, 300, 280, 512, u64::MAX).unwrap();
    assert!(a.to_canvas_bytes() == b.to_canvas_bytes());
}
