//! 色バケツの処理待ちと塗り残しの入力。確定までは文書も履歴も変更しない。
use super::{
    color::{self, Reference, Request},
    tools::paint_gate,
};
use crate::notice::Source;
use crate::{
    canvas::view::CanvasView,
    jobs::{Polled, Worker},
    state::AppState,
};
use egui::{Context, Pos2, Rect};
use std::sync::atomic::AtomicBool;
use yolu_core::{CoreError, LayerId, SelectionMask};

pub struct Drag {
    /// 通った文書の点。3D では、面の外・別のテクスチャセット・UV の外を通った所と、継ぎ目（画面の線を二分しても縮まない UV の飛び）の間を
    /// `NaN` の点で区切る（区切りをまたいで、間の線を塗り残しの範囲にしない）。
    points: Vec<(f64, f64)>,
    document: u128,
    revision: u64,
    /// 3D: 前に標本を取った画面の点（そこから今の位置までを標本する）。
    last: Option<Pos2>,
}
impl Drag {
    /// 通った文書の点（区切りは NaN の点）。
    pub fn points(&self) -> &[(f64, f64)] {
        &self.points
    }
}

/// 別のスレッドで計算している塗りつぶし（受け口を捨てると取り消す）。
pub struct Job {
    worker: Worker<Result<SelectionMask, CoreError>>,
    document: u128,
    revision: u64,
    layer: LayerId,
    style: Style,
}

pub fn request(app: &AppState, layer: LayerId, points: Vec<(f64, f64)>) -> Request {
    let mut options = app.region.color.clone();
    if app.region.sample_all {
        options.reference = Reference::Visible;
    }
    Request {
        layer,
        channel: app.m2.paint_channel,
        tolerance: app.region.tolerance,
        contiguous: app.region.contiguous,
        options,
        edit_mask: app.m2.edit_mask,
        marked: app
            .region
            .references
            .iter()
            .filter_map(|&(doc, id)| (doc == app.doc.id()).then_some(id))
            .collect(),
        points,
        radius: app.brush.radius as f64,
        budget: app
            .doc
            .source_budget_bytes()
            .min(yolu_core::selection::DEFAULT_WORKING_BUDGET_BYTES),
    }
}

struct Style {
    opacity: f64,
    erase: bool,
    mask: bool,
    reveal: bool,
    channels: Vec<yolu_core::material::ChannelPaint>,
}
impl Style {
    fn capture(app: &AppState, layer: LayerId) -> Self {
        Self {
            opacity: app.brush.opacity as f64,
            erase: app.region.erase,
            mask: app.m2.edit_mask,
            reveal: super::tools::mask_reveals(app, layer, app.region.erase),
            channels: app.paint_channels(),
        }
    }
}
fn apply(
    app: &mut AppState,
    layer: LayerId,
    style: Style,
    result: Result<SelectionMask, CoreError>,
) {
    let result = result.and_then(|mask| {
        if style.mask {
            app.doc
                .fill_mask(layer, style.opacity, Some(&mask), style.reveal)
        } else {
            app.doc.fill_material(
                layer,
                &style.channels,
                style.opacity,
                Some(&mask),
                style.erase,
            )
        }
    });
    match result {
        Ok(changed) => {
            if changed {
                app.modified = true;
                if !style.erase && !style.mask {
                    app.color.remember();
                }
            }
            if changed {
                app.info(Source::Fill, app.lang.pick("塗りました。", "Filled."));
            } else {
                app.refuse(
                    Source::Fill,
                    app.lang
                        .pick("そこには塗るものがありません。", "Nothing to fill there."),
                );
            }
        }
        Err(e) => app.notify(
            crate::notice::Kind::of_core(&e),
            Source::Fill,
            app.lang.core_error(&e),
        ),
    }
}

fn refuse_busy(app: &mut AppState) -> bool {
    if app.region.job.is_some() || app.region.leftover_drag.is_some() {
        app.refuse(Source::Fill, app.lang.pick("塗りつぶし中", "Filling"));
        true
    } else {
        false
    }
}

pub fn start(app: &mut AppState, points: Vec<(f64, f64)>) {
    if refuse_busy(app) {
        return;
    }
    let layer = match paint_gate(app) {
        Ok(id) => id,
        Err(e) => {
            app.refuse(Source::Fill, e);
            return;
        }
    };
    let style = Style::capture(app, layer);
    let req = request(app, layer, points);
    if req.options.reference == Reference::Marked
        && !req.marked.iter().any(|id| app.doc.layer(*id).is_some())
    {
        app.refuse(
            Source::Fill,
            app.lang
                .pick("参照レイヤーがありません", "No reference layers"),
        );
        return;
    }
    if app.doc.width() as u64 * app.doc.height() as u64
        <= if req.options.leftovers { 1024 } else { 65536 }
    {
        let result = color::compute(&app.doc, &req, &AtomicBool::new(false));
        apply(app, layer, style, result);
        return;
    }
    let snapshot = match app.doc.capture_snapshot() {
        Ok(d) => d,
        Err(e) => {
            app.notify(
                crate::notice::Kind::of_core(&e),
                Source::Fill,
                app.lang.core_error(&e),
            );
            return;
        }
    };
    let spawn = Worker::spawn("bucket", move |tx, cancel| {
        let _ = tx.send(color::compute(&snapshot, &req, cancel.flag()));
    });
    let Ok(worker) = spawn else {
        app.fail(
            Source::Fill,
            app.lang
                .pick("塗りつぶしを始められません。", "Cannot start the fill."),
        );
        return;
    };
    app.region.job = Some(Job {
        worker: worker.cancel_on_drop(),
        document: app.doc.id(),
        revision: app.doc.revision(),
        layer,
        style,
    });
    app.info(Source::Fill, app.lang.pick("塗りつぶし中", "Filling"));
}

pub fn poll(app: &mut AppState, ctx: &Context) {
    if app.region.job.is_none() {
        return;
    }
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        // この Esc は仕事の取消に使った（キャンバスが同じ Esc で選択範囲を解除しない）
        crate::ui::window::note_escape_taken(ctx);
        app.region.job = None;
        app.info(
            Source::Fill,
            app.lang
                .pick("塗りつぶしを取り消しました。", "Fill cancelled."),
        );
        return;
    }
    let job = app.region.job.as_ref().unwrap();
    if job.document != app.doc.id() || job.revision != app.doc.revision() {
        app.region.job = None;
        return;
    }
    match job.worker.poll() {
        Polled::Message(result) => {
            let layer = job.layer;
            let mut finished = app.region.job.take().unwrap();
            let style = std::mem::replace(&mut finished.style, Style::capture(app, layer));
            match paint_gate(app) {
                Ok(now) if now == layer => apply(app, layer, style, result),
                _ => app.warn(
                    Source::Fill,
                    app.lang
                        .pick("塗りつぶしを取り消しました。", "Fill cancelled."),
                ),
            }
        }
        Polled::Lost => {
            app.region.job = None;
            app.fail(
                Source::Fill,
                app.lang.pick("塗りつぶしに失敗しました", "Fill failed"),
            );
        }
        Polled::Empty => ctx.request_repaint_after(std::time::Duration::from_millis(16)),
    }
}

pub fn begin(app: &mut AppState, view: &CanvasView, at: Pos2) -> bool {
    if refuse_busy(app) {
        return false;
    }
    if let Err(e) = paint_gate(app) {
        app.refuse(Source::Fill, e);
        return false;
    }
    let p = view.to_canvas(at);
    if p.0 < 0.0 || p.1 < 0.0 || p.0 >= app.doc.width() as f64 || p.1 >= app.doc.height() as f64 {
        return false;
    }
    app.region.leftover_drag = Some(Drag {
        points: vec![p],
        document: app.doc.id(),
        revision: app.doc.revision(),
        last: None,
    });
    true
}

/// 3D: 塗り残しのドラッグを始める。押した点の下の面の UV が指す文書の点が最初の点。面が無ければ理由を断って始めない。
pub fn begin_surface(app: &mut AppState, rect: Rect, at: Pos2) -> bool {
    if refuse_busy(app) {
        return false;
    }
    if let Err(e) = paint_gate(app) {
        app.refuse(Source::Fill, e);
        return false;
    }
    let p = match super::tools::surface_point(app, rect, at) {
        Ok(p) => p,
        Err(miss) => {
            super::tools::refuse_miss(app, miss);
            return false;
        }
    };
    app.region.leftover_drag = Some(Drag {
        points: vec![p],
        document: app.doc.id(),
        revision: app.doc.revision(),
        last: Some(at),
    });
    true
}

/// 塗り残しのドラッグの点の上限（2D も 3D も。区切りの点も数える）。
const MAX_DRAG_POINTS: usize = 4096;
/// 3D: UV で離れた 2 つの標本の間を、画面の線で二分する回数の上限（画面の 4 点を 256 分まで）。
const BRIDGE_DEPTH: u32 = 8;
/// 3D: 二分で足した点を除いても線が動かない、と見なす UV の距離（画素）。
const BRIDGE_TOLERANCE: f64 = 0.5;

/// 3D: ドラッグが `at` へ動いた。前の位置から画面で 4 点おきに標本し、その下の面の UV が指す文書の点を足す（速く動かしても間を飛ばしにくい）。
/// 面の外・別のテクスチャセット・UV の外は足さず、その前後は線でつながない。UV でブラシの半径の 2 倍＋2 画素より離れた 2 つの標本の間は、
/// 画面の線を二分して標本を足し（`bridge`）、継ぎ目（二分しても縮まない UV の飛び）と面の外でだけ区切る。引いて見ていて画面の 4 点が UV で
/// 遠くなっても、1 つのアイランドの中の線は途切れない。
pub fn drag_surface(app: &mut AppState, rect: Rect, at: Pos2) {
    let Some(drag) = app.region.leftover_drag.as_ref() else {
        return;
    };
    let Some(from) = drag.last else {
        return;
    };
    // 前の標本（画面の点と UV の点）。最後に足した点が区切りなら、無い
    let mut previous = drag
        .points
        .last()
        .copied()
        .filter(|p| p.0.is_finite())
        .map(|p| (from, p));
    let join = 2.0 * app.brush.radius as f64 + 2.0;
    let mut added = Vec::new();
    for sample in super::tools::samples(from, at) {
        let point = super::tools::surface_point(app, rect, sample).ok();
        match (previous, point) {
            (_, None) => {
                // 面の外を通った: 次の点は新しい区切りから
                if previous.take().is_some() {
                    added.push((f64::NAN, f64::NAN));
                }
            }
            (None, Some(p)) => {
                added.push(p);
                previous = Some((sample, p));
            }
            (Some((_, last)), Some(p)) if last == p => previous = Some((sample, p)),
            (Some(a), Some(p)) => {
                let mut run = Vec::new();
                bridge(app, rect, a, (sample, p), join, BRIDGE_DEPTH, &mut run);
                push_simplified(&mut added, a.1, &run);
                previous = Some((sample, p));
            }
        }
    }
    let Some(drag) = app.region.leftover_drag.as_mut() else {
        return;
    };
    if drag.points.len() + added.len() > MAX_DRAG_POINTS {
        app.region.leftover_drag = None;
        app.view3d.stroke_ended();
        app.refuse(
            Source::Fill,
            app.lang
                .pick("ストロークが長すぎます", "Stroke is too long"),
        );
        return;
    }
    drag.points.extend(added);
    drag.last = Some(at);
}

/// 3D: 画面の点 `a.0` から `b.0` までの線の、UV の点の列を `out` に足す（`a` は足さない）。UV で `join` より離れた 2 つの標本の間は、
/// 画面の線を二分して標本を足す。二分した点が面の外（別のテクスチャセット・UV の外も）なら、そこで区切る（NaN の点）。`depth` 回二分しても
/// 縮まない飛びは、継ぎ目（別のアイランドへ移った）として区切る。同じアイランドの中なら、二分するたびに UV の間がおよそ半分になるので区切らない。
fn bridge(
    app: &mut AppState,
    rect: Rect,
    a: (Pos2, (f64, f64)),
    b: (Pos2, (f64, f64)),
    join: f64,
    depth: u32,
    out: &mut Vec<(f64, f64)>,
) {
    if distance(a.1, b.1) <= join {
        out.push(b.1);
        return;
    }
    let mid = a.0 + (b.0 - a.0) * 0.5;
    let point = (depth > 0)
        .then(|| super::tools::surface_point(app, rect, mid).ok())
        .flatten();
    let Some(point) = point else {
        out.push((f64::NAN, f64::NAN));
        out.push(b.1);
        return;
    };
    bridge(app, rect, a, (mid, point), join, depth - 1, out);
    bridge(app, rect, (mid, point), b, join, depth - 1, out);
}

fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}

/// `bridge` の点の列 `run`（`anchor` の続き）を、区切りごとに、線の形を変えない点だけにして `out` へ足す。1 つの三角形の中の画面の直線は
/// UV でも直線なので、二分で足した点のほとんどは要らない（三角形の境で曲がる所は残る）。
fn push_simplified(out: &mut Vec<(f64, f64)>, anchor: (f64, f64), run: &[(f64, f64)]) {
    let mut start = anchor;
    for (i, piece) in run.split(|p| !p.0.is_finite()).enumerate() {
        if i > 0 {
            out.push((f64::NAN, f64::NAN));
            let Some((&first, rest)) = piece.split_first() else {
                continue;
            };
            out.push(first);
            start = first;
            keep_bends(out, start, rest);
        } else {
            keep_bends(out, start, piece);
        }
        if let Some(&last) = piece.last() {
            start = last;
        }
    }
}

/// `start` に続く `points` のうち、`start` から最後の点までの折れ線の形を `BRIDGE_TOLERANCE` より変える点と、最後の点を足す（Douglas–Peucker）。
fn keep_bends(out: &mut Vec<(f64, f64)>, start: (f64, f64), points: &[(f64, f64)]) {
    let Some((&end, inner)) = points.split_last() else {
        return;
    };
    let (dx, dy) = (end.0 - start.0, end.1 - start.1);
    let length = dx.hypot(dy);
    let off = |p: &(f64, f64)| {
        if length == 0.0 {
            distance(*p, start)
        } else {
            ((p.0 - start.0) * dy - (p.1 - start.1) * dx).abs() / length
        }
    };
    if let Some((i, far)) = inner
        .iter()
        .map(off)
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(&b.1))
    {
        if far > BRIDGE_TOLERANCE {
            keep_bends(out, start, &points[..=i]);
            keep_bends(out, points[i], &points[i + 1..]);
            return;
        }
    }
    out.push(end);
}

pub fn drag(app: &mut AppState, view: &CanvasView, at: Pos2) {
    let Some(drag) = app.region.leftover_drag.as_mut() else {
        return;
    };
    let p = view.to_canvas(at);
    if drag.points.last() != Some(&p) {
        if drag.points.len() >= MAX_DRAG_POINTS {
            app.region.leftover_drag = None;
            app.canvas.stroke = None;
            app.refuse(
                Source::Fill,
                app.lang
                    .pick("ストロークが長すぎます", "Stroke is too long"),
            );
        } else {
            drag.points.push(p);
        }
    }
}
pub fn finish(app: &mut AppState, cancel: bool) -> bool {
    let Some(drag) = app.region.leftover_drag.take() else {
        return false;
    };
    if !cancel && drag.document == app.doc.id() && drag.revision == app.doc.revision() {
        start(app, drag.points);
    }
    true
}
