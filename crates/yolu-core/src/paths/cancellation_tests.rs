//! 取消の確認の順序を、専用の単一 Rayon ワーカーで決まった回数に固定する。製品ビルドには入らない。
//! 旗を「N 回目の確認」で立て、その確認で Canceled を返して後の確認へ進まないことを、確認の場所ごとに確かめる。
//! 実行の速さにも別のスレッドとの競り合いにも頼らないので、混んだ台でも結果は変わらない。
use super::*;
use crate::geometry::{SurfaceGeometry, SurfaceTriangle, DEFAULT_WELD_TOLERANCE};
use glam::{Vec2, Vec3};
use std::cell::RefCell;
use std::panic::Location;
use std::sync::atomic::{AtomicBool, Ordering};

type Site = &'static Location<'static>;

/// 確認の場所ごとの回数の期待。構造で決まる場所は `Exactly`、ブラシの式（画素の出し方）で動く場所は `AtLeast`。
#[derive(Debug, Clone, Copy)]
enum Count {
    Exactly(usize),
    AtLeast(usize),
}
use Count::{AtLeast, Exactly};

struct Schedule {
    trip: usize,
    calls: Vec<Site>,
}

thread_local! {
    static SCHEDULE: RefCell<Option<Schedule>> = const { RefCell::new(None) };
}

/// `Options::check` の頭で呼ばれる。確認の場所を順に記録し、`trip` 回目の確認の直前に旗を立てる
/// （その確認が Canceled を返す）。
pub(super) fn checkpoint(cancel: Option<&AtomicBool>, site: Site) {
    SCHEDULE.with(|schedule| {
        if let Some(s) = schedule.borrow_mut().as_mut() {
            s.calls.push(site);
            if s.calls.len() == s.trip {
                cancel
                    .expect("試験用の取消旗")
                    .store(true, Ordering::Relaxed);
            }
        }
    });
}

fn record(
    trip: usize,
    flag: &AtomicBool,
    run: &impl Fn(&AtomicBool) -> Result<Rendered, Error>,
) -> (Result<Rendered, Error>, Vec<Site>) {
    flag.store(false, Ordering::Relaxed);
    SCHEDULE.set(Some(Schedule {
        trip,
        calls: Vec::new(),
    }));
    let result = run(flag);
    (result, SCHEDULE.take().expect("記録中").calls)
}

/// 取消なしで完走して確認の場所と回数の台帳を取り、台帳の場所ごとに初め・2 回目・真ん中・最後で旗を立てて、
/// その確認で Canceled になり、それまでの確認の並びが完走と同じで、後の確認へ進まないことを見る。
/// `expected` は完走の結果から、場所ごとの回数の期待（初めて現れた順）を出す。
fn check(
    expected: impl FnOnce(&Rendered) -> Vec<Count> + Send,
    run: impl Fn(&AtomicBool) -> Result<Rendered, Error> + Send + Sync,
) {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    pool.install(|| {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                SCHEDULE.take();
            }
        }
        let _reset = Reset;
        let flag = AtomicBool::new(false);
        let (result, full) = record(usize::MAX, &flag, &run);
        let rendered = result.expect("取消なしでは完走する");
        let mut sites: Vec<(Site, usize)> = Vec::new();
        for &site in &full {
            match sites.iter_mut().find(|(s, _)| *s == site) {
                Some((_, n)) => *n += 1,
                None => sites.push((site, 1)),
            }
        }
        let counts: Vec<usize> = sites.iter().map(|(_, n)| *n).collect();
        let want = expected(&rendered);
        assert!(
            counts.len() == want.len()
                && counts.iter().zip(&want).all(|(&n, w)| match *w {
                    Exactly(m) => n == m,
                    AtLeast(m) => n >= m,
                }),
            "確認の場所ごとの回数（初めて現れた順）{counts:?} が期待 {want:?} に合う。場所: {:?}",
            sites.iter().map(|(s, _)| s.to_string()).collect::<Vec<_>>()
        );
        for &(site, _) in &sites {
            let nth: Vec<usize> = (1..=full.len())
                .filter(|&trip| full[trip - 1] == site)
                .collect();
            let mut picks = vec![nth[0], nth[nth.len() / 2], nth[nth.len() - 1]];
            picks.extend(nth.get(1));
            picks.sort_unstable();
            picks.dedup();
            for trip in picks {
                let (result, calls) = record(trip, &flag, &run);
                assert_eq!(
                    result.map(|_| ()),
                    Err(Error::Canceled),
                    "{site} の確認（全体の {trip} 回目）で途中の出力を返さない"
                );
                assert_eq!(
                    calls.len(),
                    trip,
                    "{site} の確認（全体の {trip} 回目）の後の確認へ進まない"
                );
                assert!(
                    calls[..] == full[..trip],
                    "{site} の確認（全体の {trip} 回目）までの確認の並びは完走と同じ"
                );
            }
        }
    });
}

#[test]
fn canvas_path_cancellation_stops_at_every_check_site() {
    let path = CanvasPath {
        id: 0,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 4.0,
            spacing: 0.5,
            ..Default::default()
        }),
        points: [(6.0, 10.0), (30.0, 52.0), (50.0, 12.0), (58.0, 50.0)]
            .map(|(x, y)| CanvasPoint::new(x, y, 1.0).unwrap())
            .to_vec(),
        material: None,
    };
    // 入口・Painter::new・サンプルごと・出口の 2 回。サンプルの数（最初の点を含む）は結果が持つ。
    check(
        |r| {
            assert!(r.samples > 100, "途中に十分なサンプルがある");
            vec![
                Exactly(1),
                Exactly(1),
                Exactly(r.samples),
                Exactly(1),
                Exactly(1),
            ]
        },
        |flag| {
            render_canvas(
                &path,
                &Options {
                    width: 64,
                    height: 64,
                    tile_size: 16,
                    cancel: Some(flag),
                    ..Options::default()
                },
            )
        },
    );
}

#[test]
fn surface_path_cancellation_stops_at_every_check_site() {
    let (a, b, c, d) = (Vec3::ZERO, Vec3::X, Vec3::new(1.0, 1.0, 0.0), Vec3::Y);
    let g = SurfaceGeometry::new(
        vec![
            SurfaceTriangle::new(a, b, c, Vec2::ZERO, Vec2::X, Vec2::ONE),
            SurfaceTriangle::new(a, c, d, Vec2::ZERO, Vec2::ONE, Vec2::Y),
        ],
        1,
        DEFAULT_WELD_TOLERANCE,
    )
    .unwrap();
    let path = SurfacePath {
        id: 0,
        channel: Channel::Color,
        brush: PathBrush(BrushSettings {
            radius: 0.05,
            spacing: 0.17,
            pressure_size: false,
            ..Default::default()
        }),
        points: [(1, 0.2, 0.3), (0, 0.5, 0.3), (1, 0.5, 0.3), (0, 0.1, 0.6)]
            .map(|(t, u, v)| PathPoint::new(t, u, v, 1.0).unwrap())
            .to_vec(),
        model_fingerprint: fingerprint(&g),
        material: None,
    };
    // 入口・Painter::new・区間ごと（点の数 - 1）・サンプルごと・ダブごと・ダブの画素ごと・出口の 2 回。
    // 画素ごとの回数は、ダブの画素化（hardness・縁の扱い）で動くので固定せず、各ダブが 1 画素以上塗ることだけを見る。
    check(
        |r| {
            assert!(r.dabs > 30, "途中に十分なダブがある");
            vec![
                Exactly(1),
                Exactly(1),
                Exactly(3),
                Exactly(r.samples),
                Exactly(r.dabs),
                AtLeast(r.dabs),
                Exactly(1),
                Exactly(1),
            ]
        },
        |flag| {
            render_surface(
                &path,
                &g,
                &Options {
                    width: 64,
                    height: 64,
                    tile_size: 16,
                    cancel: Some(flag),
                    ..Options::default()
                },
            )
        },
    );
}
