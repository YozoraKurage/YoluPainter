//! 公開の口からは確かめられない内部の決まり: 近傍の段の取消の確認の置き場所と、同時に動かすブロック数の式。
use super::*;
use std::cell::Cell;

const W: u32 = 20;
const H: u32 = 30;

/// 画像全体を 1 つのブロックとして `neighborhood` を呼ぶ。`trip` 回目の確認で取り消し、確認の回数を返す。
fn run(settings: Settings, trip: Option<usize>) -> (Result<Vec<u8>, Error>, usize) {
    let area = Rect::new(0, 0, W, H);
    let mut buf = Vec::new();
    for i in 0..W * H {
        buf.extend_from_slice(&[
            (i * 7) as u8,
            (i * 13) as u8,
            (i * 29) as u8,
            255 - (i % 5) as u8 * 20,
        ]);
    }
    let calls = Cell::new(0usize);
    let check = || {
        calls.set(calls.get() + 1);
        if trip == Some(calls.get()) {
            Err(Error::Cancelled)
        } else {
            Ok(())
        }
    };
    let stage = Stage::new(settings);
    let result = pixels::neighborhood(&buf, area, area, W, H, &stage, ValueType::Color, &check);
    (result, calls.get())
}

#[test]
fn blur_and_sharpen_check_every_row_of_every_pass_and_stop_at_any_check() {
    for settings in [
        Settings::GaussianBlur { radius: 1 },
        Settings::GaussianBlur { radius: 6 },
        Settings::GaussianBlur { radius: 7 },
        Settings::Sharpen {
            radius: 4,
            amount: 1.5,
            threshold: 3,
        },
    ] {
        let radius = settings.halo();
        let passes = [radius.div_ceil(3), (radius + 1) / 3, radius / 3]
            .iter()
            .filter(|&&r| r > 0)
            .count();
        let (done, total) = run(settings.clone(), None);
        assert!(done.is_ok());
        // 箱ぼかしの 1 回ごとに、横の全行と縦の全行を 1 行ずつ確認し、最後に出力の全行を確認する。
        // 縦パス・横パス・出力のどれかの確認を消すと、この下限を割る。
        let rows = (H as usize) * (2 * passes + 1);
        assert!(total >= rows, "{settings:?}: {total} < {rows}");
        for n in 1..=total {
            let (result, calls) = run(settings.clone(), Some(n));
            assert_eq!(result, Err(Error::Cancelled), "{settings:?} {n}");
            assert_eq!(calls, n, "取り消した確認より先へ進んだ: {settings:?} {n}");
        }
    }
}

#[test]
fn worker_count_follows_free_budget_and_pool_size() {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap();
    pool.install(|| {
        assert_eq!(worker_count(0, 100), 1);
        assert_eq!(worker_count(99, 100), 1);
        assert_eq!(worker_count(100, 100), 1);
        assert_eq!(worker_count(250, 100), 2);
        assert_eq!(worker_count(399, 100), 3);
        assert_eq!(worker_count(10_000, 100), 4);
        assert_eq!(worker_count(10_000, 0), 4);
    });
}
