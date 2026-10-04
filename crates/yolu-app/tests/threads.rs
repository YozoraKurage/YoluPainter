//! 起動の CPU のスレッドの設定（`settings::apply_thread_setting`）: 設定のファイルの値が rayon の全体のスレッドプールに入る。
//! rayon の全体のプールは 1 つのプロセスで 1 度しか作れないので、この試験はこのファイルに 1 つだけ置く（試験の実行ファイルは別プロセス）。

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
    // rayon の数を聞くと、聞いた時点で全体のプールが既定の数で作られてしまう。既定の数は OS に聞く
    let default_threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let wanted: u32 = if default_threads == 3 { 2 } else { 3 };
    settings::save(&path, &Settings { cpu_threads: Some(wanted), ..Settings::default() }).unwrap();
    assert_eq!(settings::load(&path).0.cpu_threads, Some(wanted));
    // プールがまだ使われていないので、設定の数になる（使われたあとは変えられないので、最初の 1 回）
    settings::apply_thread_setting();
    assert_eq!(rayon::current_num_threads(), wanted as usize);
    // 2 回目は何もしない（作れない。壊れず、数も変わらない）
    settings::save(&path, &Settings { cpu_threads: Some(5), ..Settings::default() }).unwrap();
    settings::apply_thread_setting();
    assert_eq!(rayon::current_num_threads(), wanted as usize);
    let _ = std::fs::remove_dir_all(dir);
}
