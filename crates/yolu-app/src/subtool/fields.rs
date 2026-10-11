//! サブツールの欄の宣言の表。ツールごとに、プリセットが持つ設定の欄（`Field`: 名前・種類・既定・読む関数・書く関数）と、組み込みのプリセット
//! （名前と、既定から変える欄だけ）を持つ。保存の形式（`store`）・一覧の「変更あり」の判定・既定のプリセットは、この表だけから作る。
//! 欄を足すときは、ここに 1 行（と、必要なら組み込みのプリセット）を足す。`get` で読んだ値を `set` に渡すと同じ状態に戻ること
//! （`set` が値を整えるツールは、整えたあとの値を `get` が返すこと）を、試験が確かめる。

use yolu_core::geometry::SurfaceRegionKind;
use yolu_core::material::GradientShape;
use yolu_core::{LiquifyMode, RulerKind};

use super::{Builtin, Field, Kind, Value};
use crate::drafting::Figure;
use crate::gradient::End;
use crate::region::color::{Distance, Reference};
use crate::state::{AppState, Tool};
use crate::transform::advanced::Kind as TransformKind;

fn flag(v: &Value) -> bool {
    matches!(v, Value::Bool(true))
}
fn int(v: &Value) -> i32 {
    match v {
        Value::Int(n) => *n,
        _ => 0,
    }
}
fn float(v: &Value) -> f32 {
    match v {
        Value::Float(n) => *n,
        _ => 0.0,
    }
}
fn choice(v: &Value) -> &'static str {
    match v {
        Value::Choice(s) => s,
        _ => "",
    }
}

// ───────── 範囲の種類・参照・色差の名前 ─────────

const KIND_IDS: [&str; 4] = ["triangle", "mesh_part", "uv_island", "material"];

fn kind_id(kind: SurfaceRegionKind) -> &'static str {
    match kind {
        SurfaceRegionKind::Triangle => "triangle",
        SurfaceRegionKind::MeshPart => "mesh_part",
        SurfaceRegionKind::UvIsland => "uv_island",
        SurfaceRegionKind::Material => "material",
    }
}

fn kind_from(id: &str) -> SurfaceRegionKind {
    match id {
        "triangle" => SurfaceRegionKind::Triangle,
        "mesh_part" => SurfaceRegionKind::MeshPart,
        "material" => SurfaceRegionKind::Material,
        _ => SurfaceRegionKind::UvIsland,
    }
}

fn set_region_kind(app: &mut AppState, v: &Value) {
    app.region.kind = kind_from(choice(v));
    app.region.hover = None;
}

const REFERENCE_IDS: [&str; 3] = ["editing", "visible", "marked"];

fn reference_id(r: Reference) -> &'static str {
    match r {
        Reference::Editing => "editing",
        Reference::Visible => "visible",
        Reference::Marked => "marked",
    }
}

// ───────── 欄の表 ─────────

const REGION_KIND: Field = Field {
    key: "kind",
    kind: Kind::Choice(&KIND_IDS),
    default: Value::Choice("uv_island"),
    get: |a| Value::Choice(kind_id(a.region.kind)),
    set: set_region_kind,
};

static FILL: [Field; 11] = [
    Field {
        key: "by_color",
        kind: Kind::Bool,
        default: Value::Bool(false),
        get: |a| Value::Bool(a.region.by_color),
        set: |a, v| {
            a.region.by_color = flag(v);
            a.region.hover = None;
        },
    },
    REGION_KIND,
    Field {
        key: "tolerance",
        kind: Kind::Int(0, 255),
        default: Value::Int(32),
        get: |a| Value::Int(a.region.tolerance as i32),
        set: |a, v| a.region.tolerance = int(v).clamp(0, 255) as u8,
    },
    Field {
        key: "contiguous",
        kind: Kind::Bool,
        default: Value::Bool(true),
        get: |a| Value::Bool(a.region.contiguous),
        set: |a, v| a.region.contiguous = flag(v),
    },
    Field {
        key: "reference",
        kind: Kind::Choice(&REFERENCE_IDS),
        default: Value::Choice("editing"),
        get: |a| Value::Choice(reference_id(a.region.color.reference)),
        set: |a, v| {
            let reference = match choice(v) {
                "visible" => Reference::Visible,
                "marked" => Reference::Marked,
                _ => Reference::Editing,
            };
            a.region.color.reference = reference;
            a.region.sample_all = reference == Reference::Visible;
        },
    },
    Field {
        key: "distance",
        kind: Kind::Choice(&["rgb", "perceptual"]),
        default: Value::Choice("rgb"),
        get: |a| {
            Value::Choice(match a.region.color.distance {
                Distance::Rgb => "rgb",
                Distance::Perceptual => "perceptual",
            })
        },
        set: |a, v| {
            a.region.color.distance = if choice(v) == "perceptual" {
                Distance::Perceptual
            } else {
                Distance::Rgb
            }
        },
    },
    Field {
        key: "gap",
        kind: Kind::Int(0, 32),
        default: Value::Int(0),
        get: |a| Value::Int(a.region.color.gap as i32),
        set: |a, v| a.region.color.gap = int(v).clamp(0, 32) as u8,
    },
    Field {
        key: "margin",
        kind: Kind::Int(-200, 200),
        default: Value::Int(0),
        get: |a| Value::Int(a.region.color.margin as i32),
        set: |a, v| a.region.color.margin = int(v).clamp(-200, 200) as i16,
    },
    Field {
        key: "leftovers",
        kind: Kind::Bool,
        default: Value::Bool(false),
        get: |a| Value::Bool(a.region.color.leftovers),
        set: |a, v| a.region.color.leftovers = flag(v),
    },
    Field {
        key: "max_area",
        kind: Kind::Int(1, 65536),
        default: Value::Int(1024),
        get: |a| Value::Int(a.region.color.max_area as i32),
        set: |a, v| a.region.color.max_area = int(v).clamp(1, 65536) as u32,
    },
    Field {
        key: "snap_symmetry",
        kind: Kind::Bool,
        default: Value::Bool(true),
        get: |a| Value::Bool(a.region.snap_symmetry),
        set: |a, v| a.region.snap_symmetry = flag(v),
    },
];

static POLYGON_FILL: [Field; 1] = [REGION_KIND];

static GRADIENT: [Field; 2] = [
    Field {
        key: "shape",
        kind: Kind::Choice(&["linear", "radial"]),
        default: Value::Choice("linear"),
        get: |a| {
            Value::Choice(match a.gradient.shape {
                GradientShape::Linear => "linear",
                GradientShape::Radial => "radial",
            })
        },
        set: |a, v| {
            a.gradient.shape = if choice(v) == "radial" {
                GradientShape::Radial
            } else {
                GradientShape::Linear
            }
        },
    },
    Field {
        key: "end",
        kind: Kind::Choice(&["transparent", "sub"]),
        default: Value::Choice("transparent"),
        get: |a| {
            Value::Choice(match a.gradient.end {
                End::Transparent => "transparent",
                End::Sub => "sub",
            })
        },
        set: |a, v| {
            a.gradient.end = if choice(v) == "sub" {
                End::Sub
            } else {
                End::Transparent
            }
        },
    },
];

static SHAPE: [Field; 3] = [
    Field {
        key: "figure",
        kind: Kind::Choice(&["line", "rectangle", "ellipse"]),
        default: Value::Choice("line"),
        get: |a| {
            Value::Choice(match a.drafting.figure {
                Figure::Line => "line",
                Figure::Rectangle => "rectangle",
                Figure::Ellipse => "ellipse",
            })
        },
        set: |a, v| {
            a.drafting.figure = match choice(v) {
                "rectangle" => Figure::Rectangle,
                "ellipse" => Figure::Ellipse,
                _ => Figure::Line,
            };
            // 直線は塗れない（オプションバーの図形のボタンと同じ決まり）
            if a.drafting.figure == Figure::Line {
                a.drafting.fill = false;
            }
        },
    },
    Field {
        key: "fill",
        kind: Kind::Bool,
        default: Value::Bool(false),
        get: |a| Value::Bool(a.drafting.fill),
        // 図形の欄のあとで書く（直線なら塗らない）
        set: |a, v| a.drafting.fill = flag(v) && a.drafting.figure != Figure::Line,
    },
    Field {
        key: "corner",
        kind: Kind::Float(0.0, 256.0),
        default: Value::Float(0.0),
        get: |a| Value::Float(a.drafting.corner),
        set: |a, v| a.drafting.corner = float(v).clamp(0.0, 256.0),
    },
];

static RULER: [Field; 7] = [
    Field {
        key: "kind",
        kind: Kind::Choice(&["line", "parallel", "concentric", "perspective", "symmetry"]),
        default: Value::Choice("line"),
        get: |a| {
            Value::Choice(match a.rulers.kind {
                RulerKind::Line => "line",
                RulerKind::Parallel => "parallel",
                RulerKind::Concentric => "concentric",
                RulerKind::Perspective => "perspective",
                RulerKind::Symmetry => "symmetry",
            })
        },
        // これから作る定規の種類（置いてある定規は替えない）
        set: |a, v| {
            a.rulers.kind = match choice(v) {
                "parallel" => RulerKind::Parallel,
                "concentric" => RulerKind::Concentric,
                "perspective" => RulerKind::Perspective,
                "symmetry" => RulerKind::Symmetry,
                _ => RulerKind::Line,
            };
        },
    },
    Field {
        key: "two_points",
        kind: Kind::Bool,
        default: Value::Bool(false),
        get: |a| Value::Bool(a.rulers.two_points),
        set: |a, v| a.rulers.two_points = flag(v),
    },
    // 線対称を線の本数より先に書く（本数は、線対称なら偶数に整える）
    Field {
        key: "line_symmetry",
        kind: Kind::Bool,
        default: Value::Bool(true),
        get: |a| Value::Bool(a.rulers.line_symmetry),
        set: |a, v| a.rulers.set_line_symmetry(flag(v)),
    },
    Field {
        key: "lines",
        kind: Kind::Int(2, 16),
        default: Value::Int(2),
        get: |a| Value::Int(i32::from(a.rulers.lines)),
        set: |a, v| a.rulers.set_lines(int(v).clamp(2, 16) as u8),
    },
    Field {
        key: "angle_step",
        kind: Kind::Bool,
        default: Value::Bool(false),
        get: |a| Value::Bool(a.rulers.angle_step),
        set: |a, v| a.rulers.angle_step = flag(v),
    },
    Field {
        key: "step",
        kind: Kind::Int(1, 90),
        default: Value::Int(crate::rulers::DEFAULT_STEP as i32),
        get: |a| Value::Int(a.rulers.step as i32),
        set: |a, v| a.rulers.set_step(int(v).max(0) as u32),
    },
    Field {
        key: "on_layer",
        kind: Kind::Bool,
        default: Value::Bool(true),
        get: |a| Value::Bool(a.rulers.on_layer),
        set: |a, v| a.rulers.on_layer = flag(v),
    },
];

static EYEDROPPER: [Field; 1] = [Field {
    key: "all_layers",
    kind: Kind::Bool,
    default: Value::Bool(false),
    get: |a| Value::Bool(a.eyedrop.all_layers),
    set: |a, v| a.eyedrop.all_layers = flag(v),
}];

static MOVE: [Field; 3] = [
    Field {
        key: "kind",
        kind: Kind::Choice(&["affine", "free", "perspective", "mesh"]),
        default: Value::Choice("affine"),
        get: |a| {
            Value::Choice(match a.transform.advanced.kind {
                TransformKind::Affine => "affine",
                TransformKind::Free => "free",
                TransformKind::Perspective => "perspective",
                TransformKind::Mesh => "mesh",
            })
        },
        set: |a, v| {
            a.transform.advanced.kind = match choice(v) {
                "free" => TransformKind::Free,
                "perspective" => TransformKind::Perspective,
                "mesh" => TransformKind::Mesh,
                _ => TransformKind::Affine,
            }
        },
    },
    Field {
        key: "columns",
        kind: Kind::Int(1, 32),
        default: Value::Int(4),
        get: |a| Value::Int(a.transform.advanced.columns as i32),
        set: |a, v| a.transform.advanced.columns = int(v).clamp(1, 32) as usize,
    },
    Field {
        key: "rows",
        kind: Kind::Int(1, 32),
        default: Value::Int(4),
        get: |a| Value::Int(a.transform.advanced.rows as i32),
        set: |a, v| a.transform.advanced.rows = int(v).clamp(1, 32) as usize,
    },
];

static LIQUIFY: [Field; 3] = [
    Field {
        key: "mode",
        kind: Kind::Choice(&[
            "push",
            "clockwise",
            "counter_clockwise",
            "pinch",
            "expand",
            "restore",
        ]),
        default: Value::Choice("push"),
        get: |a| {
            Value::Choice(match a.transform.advanced.mode {
                LiquifyMode::Push => "push",
                LiquifyMode::Clockwise => "clockwise",
                LiquifyMode::CounterClockwise => "counter_clockwise",
                LiquifyMode::Pinch => "pinch",
                LiquifyMode::Expand => "expand",
                LiquifyMode::Restore => "restore",
            })
        },
        set: |a, v| {
            a.transform.advanced.mode = match choice(v) {
                "clockwise" => LiquifyMode::Clockwise,
                "counter_clockwise" => LiquifyMode::CounterClockwise,
                "pinch" => LiquifyMode::Pinch,
                "expand" => LiquifyMode::Expand,
                "restore" => LiquifyMode::Restore,
                _ => LiquifyMode::Push,
            }
        },
    },
    Field {
        key: "diameter",
        kind: Kind::Float(1.0, 2048.0),
        default: Value::Float(80.0),
        get: |a| Value::Float(a.transform.advanced.diameter as f32),
        set: |a, v| a.transform.advanced.diameter = float(v).clamp(1.0, 2048.0) as f64,
    },
    Field {
        key: "strength",
        kind: Kind::Float(0.0, 1.0),
        default: Value::Float(0.5),
        get: |a| Value::Float(a.transform.advanced.strength as f32),
        set: |a, v| a.transform.advanced.strength = float(v).clamp(0.0, 1.0) as f64,
    },
];

/// ツールのプリセットが持つ欄（プリセットの値は、この並びの `Vec<Value>`）。プリセットのツールでなければ空。
pub fn fields(tool: Tool) -> &'static [Field] {
    match tool {
        Tool::Fill => &FILL,
        Tool::PolygonFill => &POLYGON_FILL,
        Tool::Gradient => &GRADIENT,
        Tool::Shape => &SHAPE,
        Tool::Ruler => &RULER,
        Tool::Eyedropper => &EYEDROPPER,
        Tool::Move => &MOVE,
        Tool::Liquify => &LIQUIFY,
        _ => &[],
    }
}

/// プリセットの欄を持つツール（この並びで状態に 1 つずつ一覧を持つ）。
pub const TOOLS: [Tool; 8] = [
    Tool::Fill,
    Tool::PolygonFill,
    Tool::Gradient,
    Tool::Shape,
    Tool::Ruler,
    Tool::Eyedropper,
    Tool::Move,
    Tool::Liquify,
];

const fn builtin(
    id: &'static str,
    ja: &'static str,
    en: &'static str,
    with: &'static [(&'static str, Value)],
) -> Builtin {
    Builtin { id, ja, en, with }
}

static FILL_BUILTINS: [Builtin; 5] = [
    builtin(
        "similar-colors",
        "近い色",
        "Similar colors",
        &[("by_color", Value::Bool(true))],
    ),
    builtin(
        "triangle",
        "三角形",
        "Triangle",
        &[("kind", Value::Choice("triangle"))],
    ),
    builtin(
        "mesh-part",
        "メッシュの塊",
        "Mesh Part",
        &[("kind", Value::Choice("mesh_part"))],
    ),
    builtin("uv-island", "UV アイランド", "UV Island", &[]),
    builtin(
        "material",
        "マテリアル",
        "Material",
        &[("kind", Value::Choice("material"))],
    ),
];

static POLYGON_FILL_BUILTINS: [Builtin; 4] = [
    builtin(
        "triangle",
        "三角形",
        "Triangle",
        &[("kind", Value::Choice("triangle"))],
    ),
    builtin(
        "mesh-part",
        "メッシュの塊",
        "Mesh Part",
        &[("kind", Value::Choice("mesh_part"))],
    ),
    builtin("uv-island", "UV アイランド", "UV Island", &[]),
    builtin(
        "material",
        "マテリアル",
        "Material",
        &[("kind", Value::Choice("material"))],
    ),
];

static GRADIENT_BUILTINS: [Builtin; 4] = [
    builtin("linear", "線形・透明へ", "Linear to transparent", &[]),
    builtin(
        "linear-sub",
        "線形・サブの色へ",
        "Linear to sub color",
        &[("end", Value::Choice("sub"))],
    ),
    builtin(
        "radial",
        "放射・透明へ",
        "Radial to transparent",
        &[("shape", Value::Choice("radial"))],
    ),
    builtin(
        "radial-sub",
        "放射・サブの色へ",
        "Radial to sub color",
        &[
            ("shape", Value::Choice("radial")),
            ("end", Value::Choice("sub")),
        ],
    ),
];

static SHAPE_BUILTINS: [Builtin; 3] = [
    builtin("line", "直線", "Line", &[]),
    builtin(
        "rectangle",
        "長方形",
        "Rectangle",
        &[("figure", Value::Choice("rectangle"))],
    ),
    builtin(
        "ellipse",
        "楕円",
        "Ellipse",
        &[("figure", Value::Choice("ellipse"))],
    ),
];

static RULER_BUILTINS: [Builtin; 7] = [
    builtin("line", "直線定規", "Straight Ruler", &[]),
    builtin(
        "parallel",
        "平行線",
        "Parallel",
        &[("kind", Value::Choice("parallel"))],
    ),
    builtin(
        "concentric",
        "同心円",
        "Concentric",
        &[("kind", Value::Choice("concentric"))],
    ),
    builtin(
        "perspective-1",
        "パース（1 点）",
        "Perspective (1 Point)",
        &[("kind", Value::Choice("perspective"))],
    ),
    builtin(
        "perspective-2",
        "パース（2 点）",
        "Perspective (2 Points)",
        &[
            ("kind", Value::Choice("perspective")),
            ("two_points", Value::Bool(true)),
        ],
    ),
    builtin(
        "symmetry-lines",
        "対称定規",
        "Symmetry Ruler",
        &[("kind", Value::Choice("symmetry"))],
    ),
    builtin(
        "symmetry-rotation",
        "回転対称",
        "Rotational Symmetry",
        &[
            ("kind", Value::Choice("symmetry")),
            ("line_symmetry", Value::Bool(false)),
            ("lines", Value::Int(6)),
        ],
    ),
];

static EYEDROPPER_BUILTINS: [Builtin; 2] = [
    builtin("layer", "選んだレイヤー", "Selected Layer", &[]),
    builtin(
        "all",
        "全レイヤー",
        "All Layers",
        &[("all_layers", Value::Bool(true))],
    ),
];

static MOVE_BUILTINS: [Builtin; 4] = [
    builtin("affine", "通常", "Normal", &[]),
    builtin("free", "自由", "Free", &[("kind", Value::Choice("free"))]),
    builtin(
        "perspective",
        "遠近",
        "Perspective",
        &[("kind", Value::Choice("perspective"))],
    ),
    builtin(
        "mesh",
        "メッシュ",
        "Mesh",
        &[("kind", Value::Choice("mesh"))],
    ),
];

static LIQUIFY_BUILTINS: [Builtin; 6] = [
    builtin("push", "プッシュ", "Push", &[]),
    builtin(
        "clockwise",
        "右回転",
        "Twirl Right",
        &[("mode", Value::Choice("clockwise"))],
    ),
    builtin(
        "counter-clockwise",
        "左回転",
        "Twirl Left",
        &[("mode", Value::Choice("counter_clockwise"))],
    ),
    builtin(
        "pinch",
        "縮小",
        "Pinch",
        &[("mode", Value::Choice("pinch"))],
    ),
    builtin(
        "expand",
        "膨張",
        "Expand",
        &[("mode", Value::Choice("expand"))],
    ),
    builtin(
        "restore",
        "戻す",
        "Restore",
        &[("mode", Value::Choice("restore"))],
    ),
];

/// ツールの組み込みのプリセット（一覧の先頭からこの並び。消せない）。
pub fn builtins(tool: Tool) -> &'static [Builtin] {
    match tool {
        Tool::Fill => &FILL_BUILTINS,
        Tool::PolygonFill => &POLYGON_FILL_BUILTINS,
        Tool::Gradient => &GRADIENT_BUILTINS,
        Tool::Shape => &SHAPE_BUILTINS,
        Tool::Ruler => &RULER_BUILTINS,
        Tool::Eyedropper => &EYEDROPPER_BUILTINS,
        Tool::Move => &MOVE_BUILTINS,
        Tool::Liquify => &LIQUIFY_BUILTINS,
        _ => &[],
    }
}
