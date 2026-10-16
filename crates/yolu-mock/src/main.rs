//! UI を作る前の確かめのための、開発用の入口（配布するアプリには入らない）。
//!
//!   cargo run -p yolu-mock -- --list
//!   cargo run -p yolu-mock -- --panel layers [--lang ja|en] [--size 360x720] [--scene empty|layers]
//!   cargo run -p yolu-mock -- --sketch example-layers-b [--png out.png]
//!   cargo run -p yolu-mock -- --gallery [--png out.png]
//!
//! - `--list`: 本物のパネル（ドックのタブ。`panel <名前>`）・下書き（`sketch <名前>`）・見本帳（`gallery`）を 1 行に 1 つ出して終わる。
//! - `--panel <名前>`: 本物のパネルだけを入れたウィンドウを開く。アプリ全体（`YoluApp`）をタブ 1 つのドックで起こすので、配色・フォント・
//!   サムネイルは本物と同じ。ダミーの状態は `--scene`（`empty` は空の文書、`layers`（既定）はそれにレイヤー・グループ・マスクを足した物）。
//! - `--sketch <名前>`: 作る予定の UI を、本物の部品とダミーの値だけで描いた下書き（`sketches/`）。本物のパネルと同じドックの見出しと枠で囲む。
//!   ウィンドウで触ると値は動く（アプリの状態にはつながない）。
//! - `--gallery`: 今ある部品（`yolu_app::ui`）と、本物のパネルがよく使う並びの見本帳。下書きは、ここから部品を選んで描く。
//! - `--png <出力.png>`: ウィンドウを開かず、画面に出す描画器（wgpu）で 1 枚撮って PNG に書いて終わる。見本帳は高さを中身に合わせる。
//! - `--lang`: 言語（既定は ja）。`--size <幅>x<高さ>`: ウィンドウの大きさ（論理の点。パネルと下書きは 360x720、見本帳は 640x900）。
//!   `--scale`: PNG の拡大率（既定は 1）。
//!
//! 設定・キーの設定・復旧・ログのファイルは読まず、書かない。アセットの欄のライブラリは、走らせるたびに `std::env::temp_dir()` の下に
//! 新しく作る空のフォルダを指し（ここへは書く。終わったら消す）、個人のライブラリには触れない。ペン・.ylp・モデルの読み込みは無い。

mod args;
mod gallery;
mod page;
mod panel;
mod scene;
mod shoot;
mod sketches;

use std::process::ExitCode;

use yolu_app::app::Tab;

use args::{Command, Options, Target};
use page::{Draw, SketchApp, SketchCtx};
use scene::TempLibrary;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let command = match args::parse(&args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!(
                "{message}\n{}\nrun with --list to see the names",
                args::USAGE
            );
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::List => {
            print_list();
            Ok(())
        }
        Command::Show(options) => {
            TempLibrary::create().and_then(|library| show(&options, &library.0))
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// 出せる物の名前。1 行に 1 つ、先頭の語が種類（`panel`・`sketch`・`gallery`）。
fn print_list() {
    for tab in Tab::ALL {
        println!("panel {}", tab.key());
    }
    for sketch in sketches::SKETCHES {
        println!("sketch {}", sketch.name);
    }
    println!("gallery");
}

/// 下書きと見本帳の題と描く関数。
fn page_of(target: Target, options: &Options) -> Option<(String, Draw)> {
    match target {
        Target::Panel(_) => None,
        Target::Sketch(name) => sketches::find(name).map(|s| {
            (
                options.lang.pick(s.title.0, s.title.1).to_owned(),
                s.draw as Draw,
            )
        }),
        Target::Gallery => Some((
            options.lang.pick("見本帳", "Gallery").to_owned(),
            gallery::draw as Draw,
        )),
    }
}

fn show(options: &Options, library: &std::path::Path) -> Result<(), String> {
    let (owned, library) = (options.clone(), library.to_path_buf());
    let png = options.png.clone();
    if let Some((title, draw)) = page_of(options.target, options) {
        let fit = options.target == Target::Gallery;
        let build = move |ctx: &egui::Context, _: Option<&eframe::egui_wgpu::RenderState>| {
            let still = owned.png.is_some();
            let sketch_ctx = SketchCtx::new(&owned, &library, still);
            SketchApp::new(ctx, title, draw, sketch_ctx)
        };
        return match png {
            Some(path) => shoot::write_png(options, &path, fit, build),
            None => shoot::open_window(options, build),
        };
    }
    let Target::Panel(tab) = options.target else {
        return Err("nothing to show".to_owned());
    };
    let build = move |ctx: &egui::Context, rs: Option<&eframe::egui_wgpu::RenderState>| {
        panel::build(tab, &owned, &library, ctx, rs)
    };
    match png {
        Some(path) => shoot::write_png(options, &path, false, build),
        None => shoot::open_window(options, build),
    }
}
