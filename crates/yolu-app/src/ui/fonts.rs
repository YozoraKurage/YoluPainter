//! 日本語の書体。egui の既定の書体には日本語が無いので、OS の書体を実行時に読む（同梱しない。書体のライセンスを持ち込まないため）。
//! 見つからなければ egui の既定のまま（日本語は豆腐になる）。`YOLU_UI_FONT=パス[:番号]` で差し替えられる（太字は
//! `YOLU_UI_FONT_BOLD`）。

use std::path::PathBuf;

use egui::{FontData, FontDefinitions, FontFamily};

use super::theme::BOLD;

/// 読んだ書体（試験と状態の表示用）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontReport {
    pub regular: Option<String>,
    pub bold: Option<String>,
}

fn candidates(bold: bool) -> Vec<(PathBuf, u32)> {
    let env = if bold {
        "YOLU_UI_FONT_BOLD"
    } else {
        "YOLU_UI_FONT"
    };
    let mut list = Vec::new();
    if let Ok(value) = std::env::var(env) {
        let (path, index) = match value.rsplit_once(':') {
            Some((p, i)) if i.chars().all(|c| c.is_ascii_digit()) && !i.is_empty() => {
                (p.to_owned(), i.parse().unwrap_or(0))
            }
            _ => (value.clone(), 0),
        };
        list.push((PathBuf::from(path), index));
    }
    let names: &[(&str, u32)] = if cfg!(windows) {
        if bold {
            &[
                ("C:\\Windows\\Fonts\\YuGothB.ttc", 0),
                ("C:\\Windows\\Fonts\\meiryob.ttc", 0),
                ("C:\\Windows\\Fonts\\msgothic.ttc", 0),
            ]
        } else {
            &[
                ("C:\\Windows\\Fonts\\YuGothM.ttc", 0),
                ("C:\\Windows\\Fonts\\meiryo.ttc", 0),
                ("C:\\Windows\\Fonts\\msgothic.ttc", 0),
            ]
        }
    } else if cfg!(target_os = "macos") {
        if bold {
            &[
                ("/System/Library/Fonts/ヒラギノ角ゴシック W6.ttc", 0),
                ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
            ]
        } else {
            &[
                ("/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc", 0),
                ("/System/Library/Fonts/Hiragino Sans GB.ttc", 0),
            ]
        }
    } else if bold {
        &[
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Bold.ttc", 0),
            ("/usr/share/fonts/noto-cjk/NotoSansCJK-Bold.ttc", 0),
            ("/usr/share/fonts/google-noto-cjk/NotoSansCJK-Bold.ttc", 0),
        ]
    } else {
        &[
            ("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0),
            ("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc", 0),
            (
                "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
                0,
            ),
            ("/usr/share/fonts/truetype/fonts-japanese-gothic.ttf", 0),
        ]
    };
    list.extend(names.iter().map(|(p, i)| (PathBuf::from(p), *i)));
    list
}

fn load(bold: bool) -> Option<(String, FontData)> {
    candidates(bold).into_iter().find_map(|(path, index)| {
        let bytes = std::fs::read(&path).ok()?;
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        Some((path.display().to_string(), data))
    })
}

/// 書体を入れる。日本語の書体を一番先に置き（英数字もその書体で揃える）、egui の既定の書体を後ろに残す（記号の補い）。
pub fn install(ctx: &egui::Context) -> FontReport {
    let mut fonts = FontDefinitions::default();
    let mut report = FontReport::default();
    let regular = load(false);
    let bold = load(true);
    if let Some((path, data)) = regular {
        fonts.font_data.insert("ui".into(), data.into());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            let list = fonts.families.entry(family.clone()).or_default();
            if family == FontFamily::Proportional {
                list.insert(0, "ui".into());
            } else {
                list.push("ui".into());
            }
        }
        report.regular = Some(path);
    }
    let mut bold_list: Vec<String> = Vec::new();
    if let Some((path, data)) = bold {
        fonts.font_data.insert("ui-bold".into(), data.into());
        bold_list.push("ui-bold".into());
        report.bold = Some(path);
    }
    bold_list.extend(
        fonts
            .families
            .get(&FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts
        .families
        .insert(FontFamily::Name(BOLD.into()), bold_list);
    ctx.set_fonts(fonts);
    report
}
