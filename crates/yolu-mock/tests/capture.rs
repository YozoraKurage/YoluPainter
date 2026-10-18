//! 実行ファイル `mock` を起こして、一覧・撮影・断りを通しで確かめる（撮影は画面に出す描画器を使う。使える物が無い台では、CI の
//! `YOLUPAINTER_REQUIRE_GPU` が無い限り省略する。ほかの GPU のテストと同じ決まり）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const MOCK: &str = env!("CARGO_BIN_EXE_mock");

/// 本物のパネル（ドックのタブ）の名前と順（`Tab::ALL`）。
const PANELS: [&str; 18] = [
    "subtools",
    "tool_properties",
    "brush_size",
    "assets",
    "color",
    "pose",
    "canvas",
    "view3d",
    "texture_sets",
    "layers",
    "properties",
    "material",
    "channels",
    "history",
    "color_sets",
    "navigator",
    "log",
    "actions",
];

fn run(args: &[&str]) -> Output {
    Command::new(MOCK)
        .args(args)
        .output()
        .expect("mock が起こせない")
}

fn run_in(args: &[&str], home: &Path) -> Output {
    // 設定・データ・キャッシュの場所を空のフォルダに向ける。Mesa の描画の写し（シェーダーのキャッシュ）はアプリの書く物ではないので切る
    Command::new(MOCK)
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env("MESA_SHADER_CACHE_DISABLE", "true")
        .output()
        .expect("mock が起こせない")
}

/// テストの一時フォルダ。描画器が無くて途中で戻ったときも、落ちたときも、手放すときに消す。
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for Scratch {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

impl AsRef<Path> for Scratch {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

fn scratch(tag: &str) -> Scratch {
    let dir = std::env::temp_dir().join(format!("yolu-mock-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    Scratch(dir)
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 撮れたか。描画器が無い台は `None`（`YOLUPAINTER_REQUIRE_GPU` があれば落とす）。
fn captured(output: &Output, what: &str) -> Option<()> {
    if output.status.success() {
        return Some(());
    }
    let stderr = text(&output.stderr);
    if stderr.contains("No adapter found") {
        eprintln!("省略: {what}: 使える描画器が無い。未検証: 撮影");
        assert!(
            std::env::var_os("YOLUPAINTER_REQUIRE_GPU").is_none(),
            "YOLUPAINTER_REQUIRE_GPU があるのに描画器が無い: {what}"
        );
        return None;
    }
    panic!("{what} が撮れない: {stderr}");
}

/// PNG の大きさと、異なる画素の数（2 色以上あれば空でない）。
fn read_png(path: &Path) -> (u32, u32, usize) {
    let decoder = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut buffer).unwrap();
    let bytes_per_pixel = info.color_type.samples() * (info.bit_depth as usize / 8);
    let mut colors = std::collections::HashSet::new();
    for pixel in buffer[..info.buffer_size()].chunks(bytes_per_pixel) {
        colors.insert(pixel.to_vec());
        if colors.len() > 64 {
            break;
        }
    }
    (info.width, info.height, colors.len())
}

/// `--list` の `sketch <名前>` の行の名前。
fn sketch_names() -> Vec<String> {
    let out = run(&["--list"]);
    assert!(out.status.success());
    text(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("sketch ").map(str::to_owned))
        .collect()
}

#[test]
fn list_names_the_panels_the_sketches_and_the_gallery() {
    let out = run(&["--list"]);
    assert!(out.status.success());
    let lines: Vec<String> = text(&out.stdout).lines().map(str::to_owned).collect();
    let panels: Vec<&str> = lines
        .iter()
        .filter_map(|l| l.strip_prefix("panel "))
        .collect();
    assert_eq!(panels, PANELS, "タブの名前と順は前と同じ");
    let sketches = sketch_names();
    for want in ["example-layers-a", "layers-c"] {
        assert!(sketches.iter().any(|s| s == want), "{want}");
    }
    assert!(lines.iter().any(|l| l == "gallery"));
    assert_eq!(lines.len(), PANELS.len() + sketches.len() + 1, "{lines:?}");
}

#[test]
fn bad_arguments_are_refused_with_status_2() {
    for args in [
        &["--sketch", "nope"][..],
        &["--panel", "nope"],
        &["--gallery", "--panel", "layers"],
        &[],
        &["--sketch"],
    ] {
        let out = run(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert!(text(&out.stderr).contains("usage:"), "{args:?}");
    }
}

#[test]
fn every_sketch_and_the_gallery_are_captured_and_not_blank() {
    let dir = scratch("capture");
    let mut targets: Vec<(Vec<String>, String)> = sketch_names()
        .into_iter()
        .map(|n| (vec!["--sketch".to_owned(), n.clone()], n))
        .collect();
    targets.push((vec!["--gallery".to_owned()], "gallery".to_owned()));
    for (lang, flag) in [("ja", false), ("en", true)] {
        for (args, name) in &targets {
            let png = dir.join(format!("{name}-{lang}.png"));
            let mut all: Vec<&str> = args.iter().map(String::as_str).collect();
            all.extend(["--png", png.to_str().unwrap()]);
            if flag {
                all.extend(["--lang", "en"]);
            }
            let out = run(&all);
            if captured(&out, name).is_none() {
                return;
            }
            let (width, height, colors) = read_png(&png);
            assert!(colors > 1, "{name} {lang}: 全部が同じ色");
            if name == "gallery" {
                assert_eq!(width, 640);
                assert!(height > 2000, "見本帳の高さは中身に合う: {height}");
            } else {
                assert_eq!((width, height), (360, 720), "{name}");
            }
        }
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_real_panel_is_still_captured() {
    let dir = scratch("panel");
    let png = dir.join("layers.png");
    let out = run(&["--panel", "layers", "--png", png.to_str().unwrap()]);
    if captured(&out, "layers").is_none() {
        return;
    }
    let (width, height, colors) = read_png(&png);
    assert_eq!((width, height), (360, 720));
    assert!(colors > 1);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_gallery_taller_than_the_gpu_limit_is_refused_not_a_crash() {
    let dir = scratch("tall");
    let png = dir.join("tall.png");
    // 640 点 × 8 の幅は収まるが、高さ（5000 点を超える）は 8 倍で 8192 を超える
    let out = run(&["--gallery", "--scale", "8", "--png", png.to_str().unwrap()]);
    if !out.status.success() && text(&out.stderr).contains("No adapter found") {
        return; // 描画器が無い台。上のテストが決まりどおりに扱う
    }
    assert_eq!(out.status.code(), Some(1), "{}", text(&out.stderr));
    assert!(
        text(&out.stderr).contains("too tall"),
        "{}",
        text(&out.stderr)
    );
    assert!(!png.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn capturing_writes_nothing_to_the_users_folders() {
    let dir = scratch("home");
    let home = dir.join("home");
    for sub in ["", "config", "data", "cache"] {
        std::fs::create_dir_all(home.join(sub)).unwrap();
    }
    let outputs = dir.join("out");
    std::fs::create_dir_all(&outputs).unwrap();
    for (i, args) in [
        &["--gallery"][..],
        &["--sketch", "layers-c"],
        &["--panel", "assets"],
    ]
    .iter()
    .enumerate()
    {
        let png = outputs.join(format!("{i}.png"));
        let mut all: Vec<&str> = args.to_vec();
        all.extend(["--png", png.to_str().unwrap()]);
        let out = run_in(&all, &home);
        if captured(&out, "home").is_none() {
            return;
        }
    }
    // HOME と、設定・データ・キャッシュの場所が、空のまま（サブフォルダも作られない）
    for sub in ["", "config", "data", "cache"] {
        let path = home.join(sub);
        let entries: Vec<_> = std::fs::read_dir(&path)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .filter(|n| !(sub.is_empty() && ["config", "data", "cache"].iter().any(|s| n == s)))
            .collect();
        assert!(entries.is_empty(), "{}: {entries:?}", path.display());
    }
    let _ = std::fs::remove_dir_all(dir);
}
