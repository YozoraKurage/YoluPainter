//! 下書き（作る予定の UI を、本物の部品とダミーの値だけで描いた物。操作にはつながない）。
//!
//! 追加するとき: `sketches/<名前>.rs` に `pub fn draw(ui: &mut egui::Ui, ctx: &mut SketchCtx)` を書き、下の `SKETCHES` の末尾に 1 行追加する
//! （ファイル名の `_` は、呼ぶ名前では `-`）。部品は見本帳（`--gallery`）から選ぶ。

use crate::page::Draw;

mod example_layers_a;
mod layers_c;
pub(crate) mod layers_common;
mod layers_d;

/// 登録した下書き 1 つ。
pub struct Sketch {
    /// `--sketch` で呼ぶ名前。
    pub name: &'static str,
    /// タブの見出し（日本語・英語）。
    pub title: (&'static str, &'static str),
    pub draw: Draw,
}

pub const SKETCHES: &[Sketch] = &[
    Sketch {
        name: "example-layers-a",
        title: ("レイヤー", "Layers"),
        draw: example_layers_a::draw,
    },
    Sketch {
        name: "layers-c",
        title: ("レイヤーの欄（案 C）", "Layers (option C)"),
        draw: layers_c::draw,
    },
    Sketch {
        name: "layers-d",
        title: ("レイヤーの欄（案 D）", "Layers (option D)"),
        draw: layers_d::draw,
    },
];

pub fn find(name: &str) -> Option<&'static Sketch> {
    SKETCHES.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_callable() {
        for (i, a) in SKETCHES.iter().enumerate() {
            assert!(!a.name.is_empty() && !a.name.contains(char::is_whitespace));
            assert!(
                SKETCHES[i + 1..].iter().all(|b| b.name != a.name),
                "{}",
                a.name
            );
            assert_eq!(find(a.name).map(|s| s.name), Some(a.name));
        }
        assert!(find("nope").is_none());
    }

    #[test]
    fn the_names_do_not_collide_with_the_panel_names() {
        for sketch in SKETCHES {
            assert!(yolu_app::app::Tab::from_key(sketch.name).is_none());
        }
    }
}
