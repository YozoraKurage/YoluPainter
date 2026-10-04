//! 専用の単一 Rayon ワーカーで取消確認の順序を固定する。製品ビルドには入らない。
use super::*;
use std::cell::Cell;

thread_local! {
    static SCHEDULE: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

pub(super) fn checkpoint(cancel: Option<&AtomicBool>) {
    SCHEDULE.with(|schedule| {
        if let Some((calls, trip)) = schedule.get() {
            let calls = calls + 1;
            schedule.set(Some((calls, trip)));
            if calls == trip {
                cancel
                    .expect("試験用の取消旗")
                    .store(true, Ordering::Relaxed);
            }
        }
    });
}

fn check(
    expected_checks: usize,
    trips: &[usize],
    run: impl Fn(&AtomicBool) -> Result<(), FillError> + Send + Sync,
) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    pool.install(|| {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                SCHEDULE.set(None);
            }
        }
        let _reset = Reset;
        let flag = AtomicBool::new(false);
        SCHEDULE.set(Some((0, usize::MAX)));
        assert_eq!(run(&flag), Ok(()));
        assert_eq!(
            SCHEDULE.get().unwrap().0,
            expected_checks,
            "行・段・256画素ごとの取消確認がある"
        );
        for &trip in trips {
            flag.store(false, Ordering::Relaxed);
            SCHEDULE.set(Some((0, trip)));
            assert_eq!(
                run(&flag),
                Err(FillError::Canceled),
                "確認 {trip} で途中の出力を返さない"
            );
            assert_eq!(
                SCHEDULE.get().unwrap().0,
                trip,
                "確認 {trip} の後の行・段へ進まない"
            );
        }
    });
}

#[test]
fn cancellation_during_mip_build_stops_early_without_a_chain() {
    let pixels = vec![127; 64 * 64 * 4];
    // 入口・出口 + 6段 + 各段の32+16+8+4+2+1行。
    // 4回目は最初の段の2行目、36回目は2段目の先頭行。
    check(71, &[4, 36], |flag| {
        ImageMipChain::build(
            &pixels,
            64,
            64,
            Conversion::None,
            false,
            u64::MAX,
            Some(flag),
        )
        .map(|_| ())
    });
}

#[test]
fn cancellation_during_geometry_bake_stops_early_without_maps() {
    use crate::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
    use glam::{Vec2, Vec3};
    let triangle =
        || SurfaceTriangle::new(Vec3::ZERO, Vec3::X, Vec3::Y, Vec2::ZERO, Vec2::X, Vec2::Y);
    let geometry =
        SurfaceGeometry::new(vec![triangle(), triangle()], 1, DEFAULT_WELD_TOLERANCE).unwrap();
    // 入口・出口 + 2三角形 × (三角形の入口 + 8行)。
    check(20, &[4, 12], |flag| {
        ModelMaps::from_geometry(&geometry, 8, 8, 0, u64::MAX, u64::MAX, Some(flag)).map(|_| ())
    });
}

#[test]
fn cancellation_during_render_returns_no_partial_image() {
    let pixels = vec![127; 4 * 4 * 4];
    let image =
        ImageMipChain::build(&pixels, 4, 4, Conversion::None, false, u64::MAX, None).unwrap();
    let sampler = FillSampler::bind(FillInput {
        width: 1024,
        height: 4,
        image: Some(&image),
        ..FillInput::default()
    })
    .unwrap();
    // 入口・出口 + 4行 × (行頭 + 256画素ごとに4回)。4回目は256画素を処理した後。
    check(22, &[4, 8], |flag| {
        sampler
            .render(0, 0, 1024, 4, u64::MAX, Some(flag))
            .map(|_| ())
    });
}

#[test]
fn cancellation_during_decal_values_returns_no_partial_image() {
    let sampler = FillSampler::bind(FillInput {
        width: 1024,
        height: 4,
        projection: Projection {
            mode: ProjectionMode::Decal,
            ..Projection::default()
        },
        ..FillInput::default()
    })
    .unwrap();
    let values = vec![127; 1024 * 4 * 4];
    check(22, &[4, 8], |flag| {
        sampler
            .render_decal_values(0, 0, 1024, 4, &values, u64::MAX, Some(flag))
            .map(|_| ())
    });
}
