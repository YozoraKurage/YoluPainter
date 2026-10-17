//! キャンバスの上の UI の下書き（A・B・C）が共通で使う、ダミーのテクスチャセット・レイヤーセット・チャンネルと、
//! キャンバスのダミーの絵（市松と UV の線と、今ある右上のボタン）、選ぶ一覧（メニュー）の開き方。部品は `yolu_app::ui` の物だけ。

use egui::{pos2, vec2, Color32, Painter, Rect, Stroke, Ui};
use yolu_app::ui::theme as t;
use yolu_app::ui::widgets as w;

use crate::page::SketchCtx;

pub type Name = (&'static str, &'static str);

/// テクスチャセット（汎用の名前）。
pub const SETS: [Name; 3] = [("ボディ", "Body"), ("服", "Cloth"), ("髪", "Hair")];

/// テクスチャセットごとのレイヤーセット。
pub fn layer_sets(set: usize) -> &'static [Name] {
    const BODY: [Name; 3] = [
        ("マテリアル", "Material"),
        ("影のマスク", "Shadow Mask"),
        ("下書き", "Draft"),
    ];
    const CLOTH: [Name; 2] = [("マテリアル", "Material"), ("影のマスク", "Shadow Mask")];
    const HAIR: [Name; 1] = [("マテリアル", "Material")];
    match set {
        0 => &BODY,
        1 => &CLOTH,
        _ => &HAIR,
    }
}

/// チャンネル（名前・アイコン。標準の 6 つは `m2::channel_icon` と同じ。最後はユーザーチャンネル）。
pub const CHANNELS: [(Name, &str); 7] = [
    (("カラー", "Color"), "palette"),
    (("ラフネス", "Roughness"), "blur_on"),
    (("メタリック", "Metallic"), "contrast"),
    (("ハイト", "Height"), "texture"),
    (("ノーマル", "Normal"), "3d_rotation"),
    (("エミッション", "Emission"), "light_mode"),
    (("影の強度", "Shadow Strength"), "layers"),
];

/// テクスチャセット・レイヤーセット・描くチャンネル・見せるチャンネルの今の選び。
#[derive(Clone, Copy)]
pub struct State {
    pub set: usize,
    pub layer_set: usize,
    pub paint: usize,
    pub display: usize,
}

/// 見せるチャンネルを選んでいない（描くチャンネルに付いていく）。
const FOLLOW: usize = usize::MAX;

impl State {
    pub fn read(ctx: &SketchCtx) -> State {
        let set = ctx.values.index("cb.set", 0).min(SETS.len() - 1);
        let layer_set = ctx
            .values
            .index("cb.layer-set", 0)
            .min(layer_sets(set).len() - 1);
        let paint = ctx.values.index("cb.paint", 0).min(CHANNELS.len() - 1);
        let display = match ctx.values.index("cb.display", FOLLOW) {
            FOLLOW => paint,
            d => d.min(CHANNELS.len() - 1),
        };
        State {
            set,
            layer_set,
            paint,
            display,
        }
    }
    pub fn pick_set(ctx: &mut SketchCtx, i: usize) {
        ctx.values.set_index("cb.set", i);
        ctx.values.set_index("cb.layer-set", 0);
    }
    pub fn pick_layer_set(ctx: &mut SketchCtx, i: usize) {
        ctx.values.set_index("cb.layer-set", i);
    }
    pub fn pick_paint(ctx: &mut SketchCtx, i: usize) {
        ctx.values.set_index("cb.paint", i);
    }
    pub fn pick_display(ctx: &mut SketchCtx, i: usize) {
        ctx.values.set_index("cb.display", i);
    }
}

pub const TIP_SET: Name = ("テクスチャセット", "Texture Set");
pub const TIP_LAYER_SET: Name = ("レイヤーセット", "Layer Set");
pub const TIP_PAINT: Name = ("描くチャンネル", "Paint Channel");
pub const TIP_DISPLAY: Name = ("見せるチャンネル", "Display Channel");

/// 段のアイコン（テクスチャセットに合うアイコンが無いので、名前だけ。レイヤーセットはレイヤー、描くは筆、見せるは目）。
pub const ICON_SET: Option<&str> = None;
pub const ICON_LAYER_SET: &str = "layers";
pub const ICON_PAINT: &str = "paint_brush";
pub const ICON_DISPLAY: &str = "visibility";

/// ダミーの UV のアイランド（0〜1 の正方形の中の閉じた線）。
fn islands() -> Vec<Vec<(f32, f32)>> {
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)];
    let circle = |cx: f32, cy: f32, r: f32| {
        (0..28)
            .map(|i| {
                let a = i as f32 / 28.0 * std::f32::consts::TAU;
                (cx + r * a.cos(), cy + r * a.sin())
            })
            .collect::<Vec<_>>()
    };
    vec![
        rect(0.07, 0.07, 0.47, 0.55),
        circle(0.74, 0.27, 0.19),
        rect(0.54, 0.52, 0.93, 0.68),
        rect(0.07, 0.62, 0.27, 0.93),
        rect(0.33, 0.62, 0.53, 0.93),
        rect(0.6, 0.76, 0.93, 0.93),
    ]
}

/// キャンバスの欄 1 つ分のダミー（暗い地・市松の正方形）。`uv` が Some なら UV の線をその色（不透明度込み）で重ねる。
pub fn canvas(ui: &mut Ui, view: Rect, uv: Option<Color32>) {
    let p = ui.painter_at(view);
    w::fill(&p, view, t::CANVAS_BG);
    let side = (view.width().min(view.height()) - 24.0).max(40.0);
    let square = Rect::from_center_size(view.center(), vec2(side, side));
    w::checker(&p, square, 16.0);
    if let Some(color) = uv {
        uv_lines(&p, square, color);
    }
}

fn uv_lines(p: &Painter, square: Rect, color: Color32) {
    let stroke = Stroke::new(1.0, color);
    let at = |(x, y): (f32, f32)| {
        pos2(
            square.left() + x * square.width(),
            square.top() + y * square.height(),
        )
    };
    for island in islands() {
        for i in 0..island.len() {
            p.line_segment([at(island[i]), at(island[(i + 1) % island.len()])], stroke);
        }
        // アイランドの中の三角形のように見える細い線（中心から各頂点へ）
        let n = island.len() as f32;
        let c = (
            island.iter().map(|q| q.0).sum::<f32>() / n,
            island.iter().map(|q| q.1).sum::<f32>() / n,
        );
        let faint = Stroke::new(1.0, color.gamma_multiply(0.4));
        if island.len() <= 4 {
            for q in &island {
                p.line_segment([at(c), at(*q)], faint);
            }
        }
    }
}
