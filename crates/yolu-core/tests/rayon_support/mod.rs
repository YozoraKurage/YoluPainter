//! rayon を使う仕事の固まり（デッドロック）の試験のツール: 継ぎ目の多いモデル（人工データ）・スレッド数を選べるプール・時間切れで落とす実行。
#![allow(dead_code)]
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::time::Duration;

use yolu_core::geometry::{SurfaceGeometry, SurfaceTriangle, UvTopology, DEFAULT_WELD_TOLERANCE};
use yolu_core::glam::{Vec2, Vec3};

/// n × n の四角を 3D で隣どうしにつなぎ、UV では並びを散らした n × n の升に、升の半分の大きさで置いたモデル（アイランドの間はアイランドと同じ幅）。
/// 継ぎ目の縁もアイランドも多く、アイランドの図・帯の写し・位相の対応を作る仕事が rayon で何本にも分かれる。
pub fn atlas(n: u32) -> Arc<UvTopology> {
    let mut tris = Vec::new();
    for j in 0..n {
        for i in 0..n {
            let c = u64::from(i + n * j);
            let cell = (c * 7919 + 13) % u64::from(n * n);
            let (cx, cy) = ((cell % u64::from(n)) as f32, (cell / u64::from(n)) as f32);
            let uv = |x: f32, y: f32| {
                Vec2::new(
                    (cx + 0.25 + 0.5 * x) / n as f32,
                    (cy + 0.25 + 0.5 * y) / n as f32,
                )
            };
            let p = |x: f32, y: f32| Vec3::new(i as f32 + x, j as f32 + y, 0.0);
            tris.push(SurfaceTriangle::new(
                p(0., 0.),
                p(1., 0.),
                p(1., 1.),
                uv(0., 0.),
                uv(1., 0.),
                uv(1., 1.),
            ));
            tris.push(SurfaceTriangle::new(
                p(0., 0.),
                p(1., 1.),
                p(0., 1.),
                uv(0., 0.),
                uv(1., 1.),
                uv(0., 1.),
            ));
        }
    }
    let g = SurfaceGeometry::new(tris, 1, DEFAULT_WELD_TOLERANCE).unwrap();
    Arc::new(UvTopology::new(Arc::new(g), Some(0)))
}

/// スレッドが `threads` 本の専用のプール。仕事が固まっても、巻き込まれるのはこのプールだけ（グローバルのプールを塞がない）。
/// 固まりは、プールのスレッドが多く、仕事が少ないほど出やすい（待つスレッドの空きに別の仕事を拾う）。
pub fn pool(threads: usize) -> rayon::ThreadPool {
    rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap()
}

/// `work` を別のスレッド（rayon のスレッドではない。アプリの主のスレッドや裏の仕事と同じ立場）で回し、`limit` までに終わらなければ（固まっていれば）
/// 試験を落とす。固まったスレッドは残る（止められない）ので、グローバルのプールで固まったあとは、同じ実行ファイルの rayon を使う試験が巻き込まれうる
/// （専用のプールの中で回せば、巻き込まれない）。
pub fn finishes_within<T: Send + 'static>(
    what: &str,
    limit: Duration,
    work: impl FnOnce() -> T + Send + 'static,
) -> T {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name(format!("deadlock-probe: {what}"))
        .spawn(move || {
            let _ = tx.send(work());
        })
        .unwrap();
    match rx.recv_timeout(limit) {
        Ok(v) => v,
        Err(RecvTimeoutError::Timeout) => {
            panic!("{what}: {limit:?} のうちに終わらなかった（固まっている）")
        }
        Err(RecvTimeoutError::Disconnected) => panic!("{what}: 仕事のスレッドが落ちた"),
    }
}

/// 専用のプールのスレッド数（並列の仕事の数よりずっと多い）。
pub const MANY: usize = 32;
/// 固まりの試験の時間切れ。
pub const LIMIT: Duration = Duration::from_secs(60);

/// 冷えた物（`make` が作る。作ってすぐ、何も覚えていない状態）への同じ仕事 `eval` を、3 つの立場で回し、どれも時間内に終わり、結果が同じなこと:
/// スレッドが 1 本のプールの中（並列の仕事が分かれない基準）・グローバルのプールで回す別スレッド（アプリの主のスレッドや裏の仕事と同じ立場）・
/// 専用の多スレッドのプールの中（呼ぶスレッドが rayon のスレッド）。`make` は rayon の外で回す（作る途中で冷えた物を温めない）。
/// 作ることも時間切れの中に入れる（グローバルのプールが固まったあとでも、試験が返る）。
pub fn assert_finishes_alike<D, T>(
    name: &str,
    make: impl Fn() -> D + Send + Sync + 'static,
    eval: impl Fn(&D) -> T + Send + Sync + 'static,
) where
    D: Sync + 'static,
    T: PartialEq + std::fmt::Debug + Send + 'static,
{
    let (make, eval) = (Arc::new(make), Arc::new(eval));
    let run = |what: &str, inside: Option<usize>| {
        let (make, eval) = (make.clone(), eval.clone());
        finishes_within(&format!("{name}（{what}）"), LIMIT, move || {
            let cold = make();
            match inside {
                Some(threads) => pool(threads).install(|| eval(&cold)),
                None => eval(&cold),
            }
        })
    };
    let reference = run("1 スレッドのプール", Some(1));
    let plain = run("主のスレッド", None);
    assert_eq!(plain, reference, "{name}: 主のスレッドの結果が基準と違う");
    let inside = run("多スレッドのプールの中", Some(MANY));
    assert_eq!(
        inside, reference,
        "{name}: 多スレッドのプールの中の結果が基準と違う"
    );
}
