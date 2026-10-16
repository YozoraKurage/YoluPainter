//! 引数の読み取り。

use std::path::PathBuf;

use yolu_app::app::Tab;
use yolu_app::lang::Lang;

use crate::scene::Scene;
use crate::sketches;

pub const USAGE: &str = "usage: mock --list | --panel <name> | --sketch <name> | --gallery \
[--png <out.png>] [--lang ja|en] [--size <width>x<height>] [--scale <factor>] [--scene empty|layers]";

/// 大きさ（論理の点）の上限と、拡大率の上限。
pub const MAX_POINTS: f32 = 8192.0;
pub const MAX_SCALE: f32 = 8.0;

/// 何を出すか。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// 本物のパネル（ドックのタブ）。
    Panel(Tab),
    /// 登録した下書き（`sketches::SKETCHES` の名前）。
    Sketch(&'static str),
    /// 部品の見本帳。
    Gallery,
}

impl Target {
    /// 大きさを決めなかったときの大きさ（論理の点）。
    fn default_size(self) -> [f32; 2] {
        match self {
            Target::Panel(_) | Target::Sketch(_) => [360.0, 720.0],
            Target::Gallery => [640.0, 900.0],
        }
    }

    /// ウィンドウの題に出す名前。
    pub fn label(self) -> String {
        match self {
            Target::Panel(tab) => tab.key().to_owned(),
            Target::Sketch(name) => format!("sketch {name}"),
            Target::Gallery => "gallery".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub target: Target,
    pub png: Option<PathBuf>,
    pub lang: Lang,
    pub size: [f32; 2],
    pub scale: f32,
    pub scene: Scene,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    List,
    Show(Options),
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut list = false;
    let mut target = None;
    let mut png = None;
    let mut lang = Lang::Ja;
    let mut size = None;
    let mut scale = 1.0;
    let mut scene = Scene::Layers;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| {
            args.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let mut pick = |next: Target| {
            if target.is_some_and(|t| t != next) {
                return Err("only one of --panel, --sketch and --gallery can be given".to_owned());
            }
            target = Some(next);
            Ok(())
        };
        match arg.as_str() {
            "--list" => list = true,
            "--panel" => {
                let name = value("--panel")?;
                pick(Target::Panel(
                    Tab::from_key(&name).ok_or_else(|| format!("unknown panel name: {name}"))?,
                ))?;
            }
            "--sketch" => {
                let name = value("--sketch")?;
                let sketch =
                    sketches::find(&name).ok_or_else(|| format!("unknown sketch name: {name}"))?;
                pick(Target::Sketch(sketch.name))?;
            }
            "--gallery" => pick(Target::Gallery)?,
            "--png" => png = Some(PathBuf::from(value("--png")?)),
            "--lang" => {
                lang = match value("--lang")?.as_str() {
                    "ja" => Lang::Ja,
                    "en" => Lang::En,
                    other => return Err(format!("unknown language: {other}")),
                }
            }
            "--size" => size = Some(parse_size(&value("--size")?)?),
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
    let target = target.ok_or("one of --panel, --sketch and --gallery is required")?;
    let size = size.unwrap_or_else(|| target.default_size());
    if size[0] * scale > MAX_POINTS || size[1] * scale > MAX_POINTS {
        return Err(format!(
            "size x scale is too large: {}x{} at scale {scale} (each side must be at most {MAX_POINTS})",
            size[0], size[1]
        ));
    }
    Ok(Command::Show(Options {
        target,
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
    fn list_needs_no_target() {
        assert_eq!(parse(&args(&["--list"])), Ok(Command::List));
    }

    #[test]
    fn panel_alone_uses_the_defaults() {
        assert_eq!(
            show(&["--panel", "layers"]),
            Options {
                target: Target::Panel(Tab::Layers),
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
                target: Target::Panel(Tab::ToolProperties),
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
            assert_eq!(show(&["--panel", tab.key()]).target, Target::Panel(tab));
        }
    }

    #[test]
    fn every_registered_sketch_name_is_accepted() {
        for sketch in sketches::SKETCHES {
            let options = show(&["--sketch", sketch.name]);
            assert_eq!(options.target, Target::Sketch(sketch.name));
            assert_eq!(options.size, [360.0, 720.0]);
        }
    }

    #[test]
    fn the_sketch_options_are_read() {
        let options = show(&[
            "--sketch", "layers-c", "--png", "out.png", "--lang", "en", "--size", "300x600",
            "--scale", "2",
        ]);
        assert_eq!(
            options,
            Options {
                target: Target::Sketch("layers-c"),
                png: Some(PathBuf::from("out.png")),
                lang: Lang::En,
                size: [300.0, 600.0],
                scale: 2.0,
                scene: Scene::Layers,
            }
        );
    }

    #[test]
    fn the_gallery_is_wider_than_a_panel_by_default() {
        assert_eq!(
            show(&["--gallery", "--png", "out.png"]),
            Options {
                target: Target::Gallery,
                png: Some(PathBuf::from("out.png")),
                lang: Lang::Ja,
                size: [640.0, 900.0],
                scale: 1.0,
                scene: Scene::Layers,
            }
        );
        assert_eq!(
            show(&["--gallery", "--size", "800x500"]).size,
            [800.0, 500.0]
        );
    }

    #[test]
    fn unknown_names_and_arguments_are_refused() {
        for bad in [
            &["--panel", "nope"][..],
            &["--sketch", "nope"],
            &["--panel", "layers", "--lang", "fr"],
            &["--panel", "layers", "--scene", "model"],
            &["--panel", "layers", "--bogus"],
            &["--gallery", "--bogus"],
            &["layers"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn two_targets_at_once_are_refused() {
        for bad in [
            &["--panel", "layers", "--gallery"][..],
            &["--gallery", "--sketch", "example-layers-a"],
            &["--panel", "layers", "--sketch", "example-layers-a"],
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad:?}");
        }
        // 同じ物を 2 度言うのは、そのまま
        assert!(parse(&args(&["--gallery", "--gallery"])).is_ok());
    }

    #[test]
    fn a_missing_value_or_target_is_refused() {
        for bad in [
            &[][..],
            &["--png", "out.png"],
            &["--panel"],
            &["--sketch"],
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
