//! パネルを 1 つだけダミーの状態で出す、開発用の入口。
//!
//!   cargo run -p yolu-app --example mock -- --list
//!   cargo run -p yolu-app --example mock -- --panel layers [--lang ja|en] [--size 360x720] [--scene empty|layers]
//!   cargo run -p yolu-app --example mock -- --panel layers --png out.png [--scale 2]
//!
//! - `--list`: パネル（ドックのタブ）の名前を 1 行に 1 つ出して終わる。
//! - `--panel <名前>`: そのタブだけを入れたウィンドウを開く。`--png <出力.png>` を付けると、ウィンドウを開かず、画面に出す描画器（wgpu）で
//!   1 枚撮って PNG に書いて終わる。
//! - `--lang`: 言語（既定は ja）。`--size <幅>x<高さ>`: ウィンドウの大きさ（論理の点。既定は 360x720）。`--scale`: PNG の拡大率（既定は 1）。
//! - `--scene`: ダミーの状態。`empty` は空の文書、`layers`（既定）はそれにレイヤー・グループ・マスクを足した物。
//!
//! アプリ全体（`YoluApp`）をタブ 1 つのドックで起こすので、配色・フォント・サムネイルは本物と同じ。キャンバスは CPU の表示に固定する。
//! 設定・キーの設定・復旧・ログのファイルは読まず、書かない。アセットの欄のライブラリは、走らせるたびに `std::env::temp_dir()` の下に
//! 新しく作る空のフォルダを指し（ここへは書く。終わったら消す）、個人のライブラリには触れない。ペン・.ylp・モデルの読み込みは無い。

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yolu_app::app::{Tab, YoluApp};
use yolu_app::engine::LayerId;
use yolu_app::lang::Lang;
use yolu_app::m2::Edit;
use yolu_app::pen::PenInput;
use yolu_app::prefs::{Pref, PrefsAction};
use yolu_app::state::{Action, AppState};

const USAGE: &str = "usage: mock --list | --panel <name> [--png <out.png>] [--lang ja|en] \
[--size <width>x<height>] [--scale <factor>] [--scene empty|layers]";

/// ダミーの文書の一辺。
const DOCUMENT_SIZE: u32 = 1024;
/// 大きさ（論理の点）の上限と、拡大率の上限。
const MAX_POINTS: f32 = 8192.0;
const MAX_SCALE: f32 = 8.0;

/// ダミーの状態。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scene {
    /// `AppState::new_in` のまま。
    Empty,
    /// 空の文書に、ラスターのレイヤー 3 つ・グループ 1 つ・マスク 1 つを足した物。
    Layers,
}

#[derive(Debug, PartialEq)]
struct Options {
    tab: Tab,
    png: Option<PathBuf>,
    lang: Lang,
    size: [f32; 2],
    scale: f32,
    scene: Scene,
}

#[derive(Debug, PartialEq)]
enum Command {
    List,
    Show(Options),
}

fn parse(args: &[String]) -> Result<Command, String> {
    let mut list = false;
    let mut tab = None;
    let mut png = None;
    let mut lang = Lang::Ja;
    let mut size = [360.0, 720.0];
    let mut scale = 1.0;
    let mut scene = Scene::Layers;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--list" => list = true,
            "--panel" => {
                let name = value("--panel")?;
                tab = Some(
                    Tab::from_key(&name).ok_or_else(|| format!("unknown panel name: {name}"))?,
                );
            }
            "--png" => png = Some(PathBuf::from(value("--png")?)),
            "--lang" => {
                lang = match value("--lang")?.as_str() {
                    "ja" => Lang::Ja,
                    "en" => Lang::En,
                    other => return Err(format!("unknown language: {other}")),
                }
            }
            "--size" => size = parse_size(&value("--size")?)?,
            "--scale" => {
                let text = value("--scale")?;
                scale = text
                    .parse::<f32>()
                    .ok()
                    .filter(|s| s.is_finite() && *s > 0.0 && *s <= MAX_SCALE)
                    .ok_or_else(|| format!("bad scale: {text}"))?;
            }
            "--scene" => {
                scene = match value("--scene")?.as_str() {
                    "empty" => Scene::Empty,
                    "layers" => Scene::Layers,
                    other => return Err(format!("unknown scene: {other}")),
                }
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if list {
        return Ok(Command::List);
    }
    let tab = tab.ok_or("--panel is required")?;
    if size[0] * scale > MAX_POINTS || size[1] * scale > MAX_POINTS {
        return Err(format!(
            "size x scale is too large: {}x{} at scale {scale} (each side must be at most {MAX_POINTS})",
            size[0], size[1]
        ));
    }
    Ok(Command::Show(Options {
        tab,
        png,
        lang,
        size,
        scale,
        scene,
    }))
}

/// `360x720` の形の大きさ。
fn parse_size(text: &str) -> Result<[f32; 2], String> {
    let bad = || format!("bad size: {text}");
    let (w, h) = text.split_once('x').ok_or_else(bad)?;
    let side = |s: &str| {
        s.parse::<f32>()
            .ok()
            .filter(|v| v.is_finite() && *v >= 1.0 && *v <= MAX_POINTS)
    };
    Ok([side(w).ok_or_else(bad)?, side(h).ok_or_else(bad)?])
}

/// ダミーの状態を作る。文書は `Action`（公開の操作）だけで足す。ライブラリの場所は `library` のフォルダに当てる。
fn scene_state(scene: Scene, lang: Lang, library: &Path) -> AppState {
    let mut state = AppState::new_in(DOCUMENT_SIZE, DOCUMENT_SIZE, lang);
    state.apply(Action::Prefs(PrefsAction::Set(Pref::LibraryFolder(Some(
        library.to_path_buf(),
    )))));
    if scene == Scene::Layers {
        // 最初のレイヤーの上に 1 つ足し、そのレイヤーをグループに入れる。グループの上にもう 1 つ足して、マスクを付ける
        state.apply(Action::NewLayer);
        state.apply(Action::M2(Edit::GroupSelected));
        state.apply(Action::NewLayer);
        let top: Option<LayerId> = state.session.selected_layer;
        if let Some(id) = top {
            state.apply(Action::M2(Edit::AddMask(id)));
            // グループにも番号が振られるので、ラスターの名前は 1・2・3 と続けて見せる
            let name = format!("{} 3", lang.pick("レイヤー", "Layer"));
            state.apply(Action::RenameLayer(id, name));
        }
    }
    state
}

/// 走らせるあいだだけある、空のフォルダ（ライブラリの置き場）。消せなくても落ちない。
struct TempLibrary(PathBuf);

impl TempLibrary {
    fn create() -> Result<TempLibrary, String> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("yolu-mock-library-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(TempLibrary(dir))
    }
}

impl Drop for TempLibrary {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// アプリ全体をタブ 1 つのドックとダミーの状態で起こす。
fn build_app(
    options: &Options,
    library: &Path,
    ctx: &egui::Context,
    render_state: Option<&eframe::egui_wgpu::RenderState>,
) -> YoluApp {
    let state = scene_state(options.scene, options.lang, library);
    let mut app = YoluApp::for_context(ctx, state, PenInput::detached());
    app.dock = egui_dock::DockState::new(vec![options.tab]);
    let mut app = app.with_render_state(render_state);
    // 撮るたびに同じ絵になるよう、キャンバスと焼く場所は CPU に固定する
    app.set_canvas_backend(yolu_app::canvas::gpu::CanvasBackend::Cpu);
    app.state.bake.backend = yolu_app::bake::BakeBackend::Cpu;
    app
}

/// ウィンドウで開く。
fn open_window(options: Options, library: &Path) -> Result<(), String> {
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(format!("YoluPainter mock: {}", options.tab.key()))
            .with_inner_size(options.size),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: yolu_app::view3d::render::wgpu_configuration(false),
        // 本体と同じく、Linux は X11 で開く
        #[cfg(target_os = "linux")]
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::x11::EventLoopBuilderExtX11;
            builder.with_x11();
        })),
        ..Default::default()
    };
    let library = library.to_path_buf();
    eframe::run_native(
        "YoluPainter mock",
        native,
        Box::new(move |cc| {
            Ok(Box::new(build_app(
                &options,
                &library,
                &cc.egui_ctx,
                cc.wgpu_render_state.as_ref(),
            )))
        }),
    )
    .map_err(|e| e.to_string())
}

/// 実際のウィンドウと同じ補間（テクスチャの補間を GPU のサンプラーに任せる）で、毎回同じ画素になるようディザは切る。
fn render_options() -> eframe::egui_wgpu::RendererOptions {
    eframe::egui_wgpu::RendererOptions {
        predictable_texture_filtering: false,
        ..eframe::egui_wgpu::RendererOptions::PREDICTABLE
    }
}

/// 製品と同じ D3D12 のシェーダーのコンパイラー（Windows）で、kittest の描画器のデバイスを作る。
fn test_renderer() -> egui_kittest::wgpu::WgpuTestRenderer {
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
        create
            .instance_descriptor
            .backend_options
            .dx12
            .shader_compiler = yolu_gpu::dx12_compiler();
    }
    egui_kittest::wgpu::WgpuTestRenderer::from_render_state(
        egui_kittest::wgpu::create_render_state(setup, render_options()),
    )
}

/// ウィンドウを開かずに撮って、PNG に書く。
fn write_png(options: Options, library: &Path, path: &Path) -> Result<(), String> {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(egui::vec2(options.size[0], options.size[1]))
        .with_pixels_per_point(options.scale)
        .with_step_dt(1.0 / 60.0)
        .with_max_steps(120)
        .renderer(test_renderer())
        .build_eframe(|cc| {
            build_app(
                &options,
                library,
                &cc.egui_ctx,
                cc.wgpu_render_state.as_ref(),
            )
        });
    // 描き直しが止まるまで進める。止まらないパネル（動き続ける物）は、上限まで進めた絵を撮る
    if let Err(e) = harness.try_run() {
        eprintln!("warning: {e}; captured the last frame");
    }
    let image = harness.render()?;
    image
        .save(path)
        .map_err(|e| format!("{}: {e}", path.display()))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let command = match parse(&args) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("{message}\n{USAGE}\nrun with --list to see the panel names");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::List => {
            for tab in Tab::ALL {
                println!("{}", tab.key());
            }
            Ok(())
        }
        Command::Show(options) => {
            TempLibrary::create().and_then(|library| match options.png.clone() {
                Some(path) => write_png(options, &library.0, &path),
                None => open_window(options, &library.0),
            })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn show(list: &[&str]) -> Options {
        match parse(&args(list)).unwrap() {
            Command::Show(options) => options,
            Command::List => panic!("list"),
        }
    }

    #[test]
    fn list_needs_no_panel() {
        assert_eq!(parse(&args(&["--list"])), Ok(Command::List));
    }

    #[test]
    fn panel_alone_uses_the_defaults() {
        assert_eq!(
            show(&["--panel", "layers"]),
            Options {
                tab: Tab::Layers,
                png: None,
                lang: Lang::Ja,
                size: [360.0, 720.0],
                scale: 1.0,
                scene: Scene::Layers,
            }
        );
    }

    #[test]
    fn every_option_is_read() {
        assert_eq!(
            show(&[
                "--panel",
                "tool_properties",
                "--png",
                "out.png",
                "--lang",
                "en",
                "--size",
                "400x800",
                "--scale",
                "1.5",
                "--scene",
                "empty",
            ]),
            Options {
                tab: Tab::ToolProperties,
                png: Some(PathBuf::from("out.png")),
                lang: Lang::En,
                size: [400.0, 800.0],
                scale: 1.5,
                scene: Scene::Empty,
            }
        );
    }

    #[test]
    fn every_listed_panel_name_is_accepted() {
        for tab in Tab::ALL {
            assert_eq!(show(&["--panel", tab.key()]).tab, tab);
        }
    }

    #[test]
    fn unknown_names_and_arguments_are_refused() {
        for bad in [
            &["--panel", "nope"][..],
            &["--panel", "layers", "--lang", "fr"],
            &["--panel", "layers", "--scene", "model"],
            &["--panel", "layers", "--bogus"],
            &["layers"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_missing_value_or_panel_is_refused() {
        for bad in [
            &[][..],
            &["--png", "out.png"],
            &["--panel"],
            &["--panel", "layers", "--png"],
            &["--panel", "layers", "--size"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_size_times_scale_over_the_gpu_limit_is_refused() {
        assert!(parse(&args(&[
            "--panel", "layers", "--size", "3000x200", "--scale", "8"
        ]))
        .is_err());
        assert!(parse(&args(&[
            "--panel", "layers", "--size", "1024x200", "--scale", "8"
        ]))
        .is_ok());
    }

    #[test]
    fn bad_sizes_and_scales_are_refused() {
        for size in [
            "360",
            "360x",
            "x720",
            "0x720",
            "-1x720",
            "axb",
            "360x720x1",
            "99999x720",
            "nanx5",
        ] {
            assert!(
                parse(&args(&["--panel", "layers", "--size", size])).is_err(),
                "{size}"
            );
        }
        for scale in ["0", "-1", "abc", "inf", "NaN", "100"] {
            assert!(
                parse(&args(&["--panel", "layers", "--scale", scale])).is_err(),
                "{scale}"
            );
        }
    }
}
