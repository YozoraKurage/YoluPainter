//! 自由・遠近・格子・ゆがみの操作。ドラッグの原点から計算し、確定まで元画素を保つ。
use super::{handle_points, Bounds, HANDLE_HIT};
use crate::{
    canvas::view::CanvasView,
    state::{AppState, StrokeSource, Tool},
};
use egui::{Modifiers, Painter, Pos2};
use yolu_core::{Homography, LiquifyDab, LiquifyMode, Warp, WarpMesh};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Affine,
    Free,
    Perspective,
    Mesh,
}
#[derive(Debug)]
pub struct State {
    pub kind: Kind,
    pub columns: usize,
    pub rows: usize,
    pub diameter: f64,
    pub strength: f64,
    pub mode: LiquifyMode,
    pub draft: Option<Draft>,
    pub(super) session: Option<Session>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            kind: Kind::Affine,
            columns: 4,
            rows: 4,
            diameter: 80.,
            strength: 0.5,
            mode: LiquifyMode::Push,
            draft: None,
            session: None,
        }
    }
}
pub(super) struct Session {
    source: yolu_core::Document,
    dabs: Vec<LiquifyDab>,
    revision: u64,
    targets: Vec<yolu_core::LayerId>,
}
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("revision", &self.revision)
            .field("targets", &self.targets)
            .field("dabs", &self.dabs.len())
            .finish_non_exhaustive()
    }
}
#[derive(Debug)]
pub struct Draft {
    pub warp: Warp,
    original: [(f64, f64); 4],
    handle: usize,
    kind: Kind,
    last: (f64, f64),
    revision: u64,
    targets: Vec<yolu_core::LayerId>,
    input_limit_exceeded: bool,
}
fn quad(b: Bounds) -> [(f64, f64); 4] {
    handle_points(b)[..4].try_into().unwrap()
}

/// ゆがみは可視画素がなくなっても開始元へ戻せる。ラスターの対象があるかで判定する。
pub(super) fn interaction_bounds(app: &mut AppState) -> Option<Bounds> {
    if app.tool == Tool::Liquify {
        (!app.transform_targets().is_empty()).then_some((
            0,
            0,
            app.doc.width() as i64,
            app.doc.height() as i64,
        ))
    } else {
        app.transform_bounds_cached()
    }
}

pub fn press(app: &mut AppState, view: &CanvasView, pos: Pos2, b: Bounds, modifiers: Modifiers) {
    if app.tool == Tool::Liquify {
        let targets = app.transform_targets();
        if app.transform.advanced.session.as_ref().is_none_or(|s| {
            s.source.id() != app.doc.id()
                || s.revision != app.doc.revision()
                || s.targets != targets
        }) {
            app.transform.advanced.session =
                app.doc.capture_snapshot().ok().map(|source| Session {
                    source,
                    dabs: Vec::new(),
                    revision: app.doc.revision(),
                    targets,
                });
        }
    } else {
        app.transform.advanced.session = None;
    }
    let state = &app.transform.advanced;
    let p = view.to_canvas(pos);
    let original = quad(b);
    let kind = if modifiers.ctrl {
        Kind::Free
    } else {
        state.kind
    };
    let (warp, handle) = if app.tool == Tool::Liquify {
        (
            Warp::Liquify(if state.mode == LiquifyMode::Push {
                Vec::new()
            } else {
                vec![LiquifyDab {
                    center: p,
                    delta: (0., 0.),
                    radius: state.diameter * 0.5,
                    strength: state.strength,
                    mode: state.mode,
                }]
            }),
            0,
        )
    } else if kind == Kind::Affine {
        return;
    } else if kind == Kind::Mesh {
        let mesh = WarpMesh::new(
            [
                b.0 as f64,
                b.1 as f64,
                (b.2 - b.0) as f64,
                (b.3 - b.1) as f64,
            ],
            state.columns,
            state.rows,
        )
        .unwrap();
        let Some(i) = mesh
            .points
            .iter()
            .position(|p| view.to_screen(p.0, p.1).distance(pos) <= HANDLE_HIT)
        else {
            return;
        };
        (Warp::Mesh(mesh), i)
    } else {
        let Some(i) = original
            .iter()
            .position(|p| view.to_screen(p.0, p.1).distance(pos) <= HANDLE_HIT)
        else {
            return;
        };
        (Warp::Projective(Homography::IDENTITY), i)
    };
    let input_limit_exceeded = matches!(&warp, Warp::Liquify(dabs) if dabs.len() + state.session.as_ref().map_or(0, |s| s.dabs.len()) > LiquifyDab::MAX_COUNT);
    app.transform.advanced.draft = Some(Draft {
        input_limit_exceeded,
        warp,
        original,
        handle,
        kind,
        last: p,
        revision: app.doc.revision(),
        targets: app.transform_targets(),
    });
}
pub fn moved(app: &mut AppState, view: &CanvasView, pos: Pos2, source: StrokeSource) {
    if app
        .transform
        .drag
        .as_ref()
        .is_none_or(|d| d.source != source)
    {
        return;
    }
    let start = app.transform.drag.as_ref().unwrap().start;
    let p = view.to_canvas(pos);
    let state = &mut app.transform.advanced;
    let Some(d) = &mut state.draft else {
        return;
    };
    if d.input_limit_exceeded {
        return;
    }
    match &mut d.warp {
        Warp::Projective(h) => {
            let mut dest = d.original;
            let delta = (p.0 - start.0, p.1 - start.1);
            dest[d.handle] = (dest[d.handle].0 + delta.0, dest[d.handle].1 + delta.1);
            if d.kind == Kind::Perspective {
                let opposite = d.handle ^ 1;
                dest[opposite] = (dest[opposite].0 - delta.0, dest[opposite].1 + delta.1);
            }
            if let Ok(next) = Homography::from_quads(d.original, dest) {
                *h = next;
            }
        }
        Warp::Mesh(m) => {
            m.points[d.handle] = (
                m.source[d.handle].0 + p.0 - start.0,
                m.source[d.handle].1 + p.1 - start.1,
            );
        }
        Warp::Liquify(dabs) => {
            let distance = (p.0 - d.last.0).hypot(p.1 - d.last.1);
            if distance < 0.01 {
                return;
            }
            let steps = (distance / (state.diameter * 0.1).max(1.))
                .ceil()
                .clamp(1., 256.) as usize;
            let retained = state.session.as_ref().map_or(0, |s| s.dabs.len());
            if retained + dabs.len() + steps > LiquifyDab::MAX_COUNT {
                d.input_limit_exceeded = true;
                return;
            }
            for i in 1..=steps {
                let t = i as f64 / steps as f64;
                dabs.push(LiquifyDab {
                    center: (
                        d.last.0 + (p.0 - d.last.0) * t,
                        d.last.1 + (p.1 - d.last.1) * t,
                    ),
                    delta: (
                        (p.0 - d.last.0) / steps as f64,
                        (p.1 - d.last.1) / steps as f64,
                    ),
                    radius: state.diameter * 0.5,
                    strength: state.strength,
                    mode: state.mode,
                });
            }
        }
    }
    d.last = p;
}
pub fn commit(app: &mut AppState) -> bool {
    let Some(d) = app.transform.advanced.draft.take() else {
        return false;
    };
    app.transform.drag = None;
    if d.input_limit_exceeded {
        app.message = app
            .lang
            .pick("ゆがみの入力数が上限です", "Liquify input limit reached")
            .into();
        return true;
    }
    if app.doc.revision() != d.revision || app.transform_targets() != d.targets || !app.can_edit() {
        return true;
    }
    let mut next_dabs = None;
    let result =
        if let (Warp::Liquify(dabs), Some(session)) = (&d.warp, &app.transform.advanced.session) {
            let mut all = session.dabs.clone();
            all.extend_from_slice(dabs);
            let result = app.doc.liquify_from_snapshot_cancellable(
                &d.targets,
                &all,
                &session.source,
                &mut || false,
            );
            if result.is_ok() {
                next_dabs = Some(all);
            }
            result
        } else {
            app.doc
                .warp_layers_cancellable(&d.targets, &d.warp, &mut || false)
        };
    if let (Some(dabs), Some(session)) = (next_dabs, &mut app.transform.advanced.session) {
        session.dabs = dabs;
        session.revision = app.doc.revision();
    }
    match result {
        Ok(changed) => {
            app.modified |= changed;
            app.transform.forget_bounds();
        }
        Err(e) => app.message = crate::matpaint::refusal_text(app.lang, &e),
    }
    true
}
pub fn paint(painter: &Painter, view: &CanvasView, app: &mut AppState) -> bool {
    if !matches!(app.tool, Tool::Move | Tool::Liquify) {
        return false;
    }
    if app.tool == Tool::Liquify {
        if let Some(p) = app.canvas.last_pointer {
            painter.circle_stroke(
                p,
                (app.transform.advanced.diameter as f32 * view.pixel_size() * 0.5).max(1.),
                egui::Stroke::new(1., egui::Color32::WHITE),
            );
        }
        return true;
    }
    let Some(b) = app.transform_bounds_cached() else {
        return false;
    };
    let state = &app.transform.advanced;
    if app.transform.drag.is_some() && state.draft.is_none() {
        return false;
    }
    let mesh = if let Some(Draft {
        warp: Warp::Mesh(m),
        ..
    }) = &state.draft
    {
        Some(m.clone())
    } else if state.kind == Kind::Mesh && app.tool == Tool::Move {
        WarpMesh::new(
            [
                b.0 as f64,
                b.1 as f64,
                (b.2 - b.0) as f64,
                (b.3 - b.1) as f64,
            ],
            state.columns,
            state.rows,
        )
        .ok()
    } else {
        None
    };
    if let Some(m) = mesh {
        for j in 0..=m.rows {
            super::canvas::outline(
                painter,
                (0..=m.columns)
                    .map(|i| {
                        let p = m.points[j * (m.columns + 1) + i];
                        view.to_screen(p.0, p.1)
                    })
                    .collect(),
            );
        }
        for i in 0..=m.columns {
            super::canvas::outline(
                painter,
                (0..=m.rows)
                    .map(|j| {
                        let p = m.points[j * (m.columns + 1) + i];
                        view.to_screen(p.0, p.1)
                    })
                    .collect(),
            );
        }
        for p in m.points {
            painter.circle_filled(view.to_screen(p.0, p.1), 3., egui::Color32::WHITE);
        }
        return true;
    }
    if let Some(Draft {
        warp: Warp::Projective(h),
        original,
        ..
    }) = &state.draft
    {
        super::canvas::outline(
            painter,
            original
                .iter()
                .chain(original.first())
                .map(|p| {
                    let p = h.apply(p.0, p.1);
                    view.to_screen(p.0, p.1)
                })
                .collect(),
        );
        return true;
    }
    false
}
