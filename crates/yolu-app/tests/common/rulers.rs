//! 定規を文書に置く試験の部品。定規はレイヤー（グループも）が持つ文書の値なので、試験は「選んでいるレイヤーに定規を置く」ところから始める。
//! 置くのはアプリの `ruler_create`（画面の定規のツールが離したときに通る道。特殊定規には印が付き、「特殊定規にスナップ」が入る）。

use yolu_app::state::AppState;
use yolu_core::glam::{DVec2, DVec3};
use yolu_core::{LayerId, Ruler, RulerId, RulerKind};

/// 選んでいるレイヤーに定規を置く（1 回の取り消し）。置いた定規の ID。
pub fn add(app: &mut AppState, ruler: Ruler) -> RulerId {
    app.rulers.on_layer = true;
    app.ruler_create(ruler)
        .unwrap_or_else(|e| panic!("定規を置けない: {e:?} {}", app.message))
}

/// 選んでいるレイヤーに 2D の定規を置く。
pub fn add_canvas(app: &mut AppState, kind: RulerKind, a: DVec2, b: DVec2) -> RulerId {
    add(app, Ruler::canvas(RulerId(1), kind, a, b))
}

/// 直線定規（`a` から `b`）。
pub fn line(app: &mut AppState, a: (f64, f64), b: (f64, f64)) -> RulerId {
    add_canvas(
        app,
        RulerKind::Line,
        DVec2::new(a.0, a.1),
        DVec2::new(b.0, b.1),
    )
}

/// 2D の対称定規（中心 `center`、最初の線は `toward` の向き）。
pub fn symmetry_2d(
    app: &mut AppState,
    center: (f64, f64),
    toward: (f64, f64),
    lines: u8,
    line_symmetry: bool,
) -> RulerId {
    let a = DVec2::new(center.0, center.1);
    let mut r = Ruler::canvas(
        RulerId(1),
        RulerKind::Symmetry,
        a,
        a + DVec2::new(toward.0, toward.1),
    );
    r.lines = lines;
    r.line_symmetry = line_symmetry;
    add(app, r)
}

/// 線対称 2 本（向き 90°）: 中心を通る縦の軸で左右に写す。
pub fn vertical(app: &mut AppState, center_x: f64) -> RulerId {
    symmetry_2d(app, (center_x, 0.0), (0.0, 1.0), 2, true)
}

/// 線対称 2 本（向き 0°）: 中心を通る横の軸で上下に写す。
pub fn horizontal(app: &mut AppState, center_y: f64) -> RulerId {
    symmetry_2d(app, (0.0, center_y), (1.0, 0.0), 2, true)
}

/// 線対称 4 本（向き 0°・90°）: 縦と横の軸で 4 つに写す。
pub fn both(app: &mut AppState, center: (f64, f64)) -> RulerId {
    symmetry_2d(app, center, (1.0, 0.0), 4, true)
}

/// 回転対称（向き 0°、`count` 個の回転）。
pub fn radial(app: &mut AppState, center: (f64, f64), count: u8) -> RulerId {
    symmetry_2d(app, center, (1.0, 0.0), count, false)
}

/// 3D の対称定規（中心 `a`、最初の線は `a + toward`、回転の軸は `up`）。
pub fn symmetry_3d(
    app: &mut AppState,
    a: DVec3,
    toward: DVec3,
    up: DVec3,
    lines: u8,
    line_symmetry: bool,
) -> RulerId {
    let mut r = Ruler::model(RulerId(1), RulerKind::Symmetry, a, a + toward, up);
    r.lines = lines;
    r.line_symmetry = line_symmetry;
    add(app, r)
}

/// 3D の鏡（`a` を通り、法線が `normal`。線対称 2 本で、回転の軸 `up` と最初の線 `d` は鏡の面の中にとり、`up × d = normal`）。
pub fn mirror_3d(app: &mut AppState, a: DVec3, normal: DVec3) -> RulerId {
    let n = normal.normalize();
    let helper = if n.x.abs() < 0.9 { DVec3::X } else { DVec3::Y };
    let d = n.cross(helper).normalize();
    let up = d.cross(n).normalize();
    symmetry_3d(app, a, d, up, 2, true)
}

/// 選んでいるレイヤーが持つ定規の数。
pub fn count(app: &AppState) -> usize {
    app.selected_layer
        .and_then(|id| app.doc.layer(id))
        .map_or(0, |l| l.rulers().len())
}

/// 文書の全部のレイヤーの定規の数。
pub fn total(app: &AppState) -> usize {
    app.doc.layers().iter().map(|l| l.rulers().len()).sum()
}

/// レイヤーの定規。
pub fn of(app: &AppState, layer: LayerId) -> Vec<Ruler> {
    app.doc
        .layer(layer)
        .map(|l| l.rulers().to_vec())
        .unwrap_or_default()
}
