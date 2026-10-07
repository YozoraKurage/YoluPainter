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
use egui::{Context, Pos2};
use std::sync::atomic::AtomicBool;
use yolu_core::{CoreError, LayerId, SelectionMask};

pub struct Drag {
    points: Vec<(f64, f64)>,
    document: u128,
    revision: u64,
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
    });
    true
}
pub fn drag(app: &mut AppState, view: &CanvasView, at: Pos2) {
    let Some(drag) = app.region.leftover_drag.as_mut() else {
        return;
    };
    let p = view.to_canvas(at);
    if drag.points.last() != Some(&p) {
        if drag.points.len() >= 4096 {
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
