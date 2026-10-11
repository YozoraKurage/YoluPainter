//! 起動の設定（ウィンドウを作る前に設定のファイルから読む値）: CPU のスレッド（`settings::apply_thread_setting`）の値が rayon の全体のスレッドプールに入る。
//! 同じ設定のフォルダの垂直同期（`settings::startup_vsync`。ウィンドウの面の同期を決める）も読む。
//! rayon の全体のプールは 1 つのプロセスで 1 度しか作れず、設定のフォルダは環境変数で決まるので、この試験はこのファイルに 1 つだけ置く（試験の実行ファイルは別プロセス）。

use yolu_app::settings::{self, Settings};

#[test]
fn the_thread_count_in_the_settings_file_sets_the_global_rayon_pool_at_startup() {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/threads-tests")
        .join(std::process::id().to_string());
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = dir.canonicalize().unwrap();
    // 設定のフォルダをこの試験の中に向ける（どの OS の決まりでも、ここの設定のファイルを読む）
    std::env::set_var("XDG_CONFIG_HOME", &dir);
    std::env::set_var("HOME", &dir);
    std::env::set_var("APPDATA", &dir);
    let path = settings::path().expect("設定のファイルの場所");
    assert!(path.starts_with(&dir), "{path:?}");
    // 垂直同期: ファイルが無い・項目が無い・読めない値は待たない。入っていれば待つ
    assert!(!settings::startup_vsync(), "ファイルが無ければ待たない");
    settings::save(&path, &Settings::default()).unwrap();
    assert!(!settings::startup_vsync(), "項目が無ければ待たない");
    std::fs::write(&path, "language=ja\nvsync=maybe\n").unwrap();
    assert!(!settings::startup_vsync(), "読めない値は待たない");
    settings::save(
        &path,
        &Settings {
            vsync: true,
            ..Settings::default()
        },
    )
    .unwrap();
    assert!(settings::startup_vsync(), "入っていれば待つ");
    // rayon の数を聞くと、聞いた時点で全体のプールが既定の数で作られてしまう。既定の数は OS に聞く
    let default_threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let wanted: u32 = if default_threads == 3 { 2 } else { 3 };
    settings::save(
        &path,
        &Settings {
            cpu_threads: Some(wanted),
            ..Settings::default()
        },
    )
    .unwrap();
    assert_eq!(settings::load(&path).0.cpu_threads, Some(wanted));
    // プールがまだ使われていないので、設定の数になる（使われたあとは変えられないので、最初の 1 回）
    settings::apply_thread_setting();
    assert_eq!(rayon::current_num_threads(), wanted as usize);
    // 2 回目は何もしない（作れない。壊れず、数も変わらない）
    settings::save(
        &path,
        &Settings {
            cpu_threads: Some(5),
            ..Settings::default()
        },
    )
    .unwrap();
    settings::apply_thread_setting();
    assert_eq!(rayon::current_num_threads(), wanted as usize);
    let _ = std::fs::remove_dir_all(dir);
}
