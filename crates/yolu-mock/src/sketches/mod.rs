//! 下書き（作る予定の UI を、本物の部品とダミーの値だけで描いた物。操作にはつながない）。
//!
//! 追加するとき: `sketches/<名前>.rs` に `pub fn draw(ui: &mut egui::Ui, ctx: &mut SketchCtx)` を書き、下の `SKETCHES` の末尾に 1 行追加する
//! （ファイル名の `_` は、呼ぶ名前では `-`）。部品は見本帳（`--gallery`）から選ぶ。

use crate::page::Draw;

mod brushdyn_c;
mod brushdyn_common;
mod canvasbar_c;
mod canvasbar_common;
mod distribute_a;
mod distribute_common;
mod dropin_c;
mod dropin_common;
mod example_layers_a;
mod hideface3_common;
mod hideface3_main;
mod hideface3_multi;
mod hideface_common;
mod layers_c;
pub(crate) mod layers_common;
mod layers_d;
mod lsets_b;
pub(crate) mod lsets_common;
mod multimat_3d;
mod multimat_a;
pub(crate) mod multimat_common;
mod multimat_dropdown;
mod multimat_list;
mod multimat_new_b;
mod multimat_new_b_menu;
pub(crate) mod multimat_new_common;
mod multiview_b;
pub(crate) mod multiview_common;
mod multiview_tab;

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
    Sketch {
        name: "lsets-b",
        title: ("レイヤーセット（案 B）", "Layer sets (option B)"),
        draw: lsets_b::draw,
    },
    Sketch {
        name: "lsets-b-pick",
        title: (
            "レイヤーセット（案 B・出力の選び）",
            "Layer sets (option B, output pick)",
        ),
        draw: lsets_b::draw_pick,
    },
    Sketch {
        name: "canvasbar-c",
        title: ("キャンバスの上の UI（案 C）", "Canvas bar (option C)"),
        draw: canvasbar_c::draw,
    },
    Sketch {
        name: "canvasbar-c-uv-off",
        title: ("キャンバスの上の UI（UV 非表示）", "Canvas bar (UV hidden)"),
        draw: canvasbar_c::draw_uv_off,
    },
    Sketch {
        name: "canvasbar-c-uv-menu",
        title: (
            "キャンバスの上の UI（UV のメニュー）",
            "Canvas bar (UV menu)",
        ),
        draw: canvasbar_c::draw_uv_menu,
    },
    Sketch {
        name: "canvasbar-c-open",
        title: (
            "キャンバスの上の UI（チャンネルの欄）",
            "Canvas bar (channel list)",
        ),
        draw: canvasbar_c::draw_open,
    },
    Sketch {
        name: "dropin-c",
        title: ("ファイルの取り込み", "Import files"),
        draw: dropin_c::draw,
    },
    Sketch {
        name: "dropin-c-menu",
        title: (
            "ファイルの取り込み・メニュー（案 C）",
            "Import files, menu open (option C)",
        ),
        draw: dropin_c::draw_menu,
    },
    Sketch {
        name: "dropin-c-menu-psd",
        title: ("入れ先の一覧・PSD の行", "Destination list, PSD row"),
        draw: dropin_c::draw_menu_psd,
    },
    Sketch {
        name: "dropin-c-ready",
        title: (
            "ファイルの取り込み・全部決まった形",
            "Import files, all set",
        ),
        draw: dropin_c::draw_ready,
    },
    Sketch {
        name: "dropin-c-psd",
        title: ("PSD の取り込み", "Import PSD"),
        draw: dropin_c::draw_psd,
    },
    Sketch {
        name: "multiview-b",
        title: ("3D ビュー（案 B）", "3D View (option B)"),
        draw: multiview_b::draw,
    },
    Sketch {
        name: "multiview-b-menu",
        title: (
            "3D ビュー（案 B・表示のメニュー）",
            "3D View (option B, shading menu)",
        ),
        draw: multiview_b::draw_menu,
    },
    Sketch {
        name: "multiview-b-window",
        title: (
            "3D ビュー（案 B・別ウィンドウ）",
            "3D View (option B, new window)",
        ),
        draw: multiview_b::draw_window,
    },
    Sketch {
        name: "multiview-b-tab",
        title: ("3D ビュー（Tab で隠した形）", "3D View (UI hidden)"),
        draw: multiview_tab::draw_two,
    },
    Sketch {
        name: "multiview-b-tab-one",
        title: (
            "3D ビュー（Tab で隠した形・1 つ）",
            "3D View (UI hidden, one)",
        ),
        draw: multiview_tab::draw_one,
    },
    Sketch {
        name: "brushdyn-c",
        title: (
            "ブラシの影響元（ブラシサイズ）",
            "Brush dynamics (Brush size)",
        ),
        draw: brushdyn_c::draw,
    },
    Sketch {
        name: "brushdyn-c-color",
        title: (
            "ブラシの影響元（色の変化）",
            "Brush dynamics (Color change)",
        ),
        draw: brushdyn_c::draw_color,
    },
    Sketch {
        name: "brushdyn-c-tip",
        title: ("ブラシの影響元（ブラシ先端）", "Brush dynamics (Brush tip)"),
        draw: brushdyn_c::draw_tip,
    },
    Sketch {
        name: "brushdyn-c-stroke",
        title: ("ブラシの影響元（ストローク）", "Brush dynamics (Stroke)"),
        draw: brushdyn_c::draw_stroke,
    },
    Sketch {
        name: "brushdyn-c-paper",
        title: ("ブラシの影響元（紙質）", "Brush dynamics (Paper)"),
        draw: brushdyn_c::draw_paper,
    },
    Sketch {
        name: "brushdyn-c-taper",
        title: ("ブラシの影響元（入り抜き）", "Brush dynamics (Taper)"),
        draw: brushdyn_c::draw_taper,
    },
    Sketch {
        name: "brushdyn-c-toolprops",
        title: (
            "ブラシの影響元（ツールプロパティ）",
            "Brush dynamics (Tool properties)",
        ),
        draw: brushdyn_c::draw_toolprops,
    },
    Sketch {
        name: "distribute-a",
        title: ("配布用に保存（案 A）", "Save for distribution (option A)"),
        draw: distribute_a::draw,
    },
    Sketch {
        name: "hideface5",
        title: ("ハイド（引き出しを開いた形）", "Hide (drawer open)"),
        draw: hideface3_main::draw_main,
    },
    Sketch {
        name: "hideface5-drawer-closed",
        title: ("ハイド（引き出しを閉じた形）", "Hide (drawer closed)"),
        draw: hideface3_main::draw_drawer_closed,
    },
    Sketch {
        name: "hideface5-zoom-out",
        title: ("ハイド（ズームアウト）", "Hide (zoomed out)"),
        draw: hideface3_main::draw_zoom_out,
    },
    Sketch {
        name: "hideface5-zoom-in",
        title: ("ハイド（ズームイン）", "Hide (zoomed in)"),
        draw: hideface3_main::draw_zoom_in,
    },
    Sketch {
        name: "hideface5-pose",
        title: (
            "ポーズ（ボーンの丸と回転のギズモ）",
            "Pose (bone dots and rotation gizmo)",
        ),
        draw: hideface3_main::draw_pose,
    },
    Sketch {
        name: "hideface5-shelf-list",
        title: ("ハイド（シェルフの一覧）", "Hide (shelf list)"),
        draw: hideface3_main::draw_shelf_list,
    },
    Sketch {
        name: "hideface5-multiview",
        title: ("ハイド（マルチ 3D ビュー）", "Hide (multi 3D view)"),
        draw: hideface3_multi::draw,
    },
    Sketch {
        name: "multimat-dropdown",
        title: (
            "プロジェクト設定（マテリアルの選び）",
            "Project configuration (material picker)",
        ),
        draw: multimat_dropdown::draw,
    },
    Sketch {
        name: "multimat-a",
        title: (
            "プロジェクト設定（案 A）",
            "Project configuration (option A)",
        ),
        draw: multimat_a::draw,
    },
    Sketch {
        name: "multimat-list",
        title: ("テクスチャセット", "Texture Sets"),
        draw: multimat_list::draw,
    },
    Sketch {
        name: "multimat-3d",
        title: ("3D ビュー", "3D View"),
        draw: multimat_3d::draw,
    },
    Sketch {
        name: "multimat-new-b",
        title: ("新規プロジェクト（案 B）", "New project (option B)"),
        draw: multimat_new_b::draw,
    },
    Sketch {
        name: "multimat-new-b-menu",
        title: (
            "新規プロジェクト（案 B・マテリアルの選び）",
            "New project (option B, material picker)",
        ),
        draw: multimat_new_b_menu::draw,
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
