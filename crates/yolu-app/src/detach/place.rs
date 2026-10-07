//! 外の窓の置き場所（純関数）。座標の決めごとは `windowpos` と同じ: 記録は点（点 = 画素 / 書いたときの拡大率）で、画面と突き合わせるときは
//! 仮想スクリーンの物理画素に直す。大きさは置く画面の拡大率で点から画素へ直す（拡大率の違う画面へ移っても、見た目の大きさを保つ）。

use egui::{Pos2, Rect};

use super::MIN_SIZE;
use crate::layout::FloatRecord;
use crate::windowpos::{Monitor, Placement, PxRect};

/// 窓の上の帯のうち、つかんで動かせるかを見る部分（点。中央の幅と高さ。窓が狭ければ窓の幅）。
const TITLE_BAND: [f32; 2] = [120.0, 24.0];

/// viewport の点を、仮想スクリーンの物理画素へ（内側の左上 + 点）× 拡大率。内側の矩形が分からなければ（試験の窓）原点を左上とする。
pub fn screen_px(inner: Option<Rect>, pixels_per_point: f32, at: Pos2) -> [f32; 2] {
    let origin = inner.map_or(Pos2::ZERO, |r| r.min);
    [
        (origin.x + at.x) * pixels_per_point,
        (origin.y + at.y) * pixels_per_point,
    ]
}

/// 仮想スクリーンの物理画素を、viewport の点へ（`screen_px` の逆）。
pub fn viewport_point(inner: Option<Rect>, pixels_per_point: f32, px: [f32; 2]) -> Pos2 {
    let origin = inner.map_or(Pos2::ZERO, |r| r.min);
    Pos2::new(
        px[0] / pixels_per_point - origin.x,
        px[1] / pixels_per_point - origin.y,
    )
}

/// 内側の矩形（点）が、画素の点を含むか。
pub fn contains_px(inner: Option<Rect>, pixels_per_point: f32, px: [f32; 2]) -> bool {
    let Some(inner) = inner else {
        return false;
    };
    inner.contains(Pos2::new(
        px[0] / pixels_per_point,
        px[1] / pixels_per_point,
    ))
}

/// 外枠の左上の画素と内側の大きさ（点）から、記録（点 + 拡大率）を作る。
pub fn record_at(outer_px: [f32; 2], size: [f32; 2], pixels_per_point: f32) -> FloatRecord {
    FloatRecord {
        position: [
            outer_px[0] / pixels_per_point,
            outer_px[1] / pixels_per_point,
        ],
        size,
        pixels_per_point,
    }
}

fn primary(monitors: &[Monitor]) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|m| m.primary)
        .or_else(|| monitors.first())
}

/// `size`（点）を画面の拡大率で画素にし、作業領域に収まるまで縮める（外の窓の最小の大きさは割らない）。
fn fitted_size(monitor: &Monitor, size: [f32; 2]) -> (i32, i32) {
    let fit = |points: f32, limit: i32, minimum: f32| {
        ((points * monitor.scale).round() as i32)
            .min(limit)
            .max((minimum * monitor.scale).round() as i32)
            .max(1)
    };
    (
        fit(size[0], monitor.work.width(), MIN_SIZE[0]),
        fit(size[1], monitor.work.height(), MIN_SIZE[1]),
    )
}

/// 窓全体が作業領域に入るよう寄せる（窓が作業領域より大きければ、左上を作業領域の左上に合わせる）。
fn shifted_into(work: PxRect, left: i32, top: i32, width: i32, height: i32) -> PxRect {
    let left = left.min(work.right - width).max(work.left);
    let top = top.min(work.bottom - height).max(work.top);
    PxRect::from_origin_size(left, top, width, height)
}

/// 外の窓を置く場所。画面が分からなければ None（記録の点のまま作る）。
/// - 窓の上の帯の中央が、今のどれかの画面の作業領域に見えていれば、窓と一番重なる画面に置く。大きさは記録の点をその画面の拡大率で直し、
///   作業領域に収まるまで縮め、位置は窓全体が作業領域に入るよう寄せる。
/// - 見えていなければ（画面を外した・配置が変わった）、主の窓（`main`。画素の矩形）の上の中央に置く（主の窓が分からなければ主の画面の中央）。
pub fn plan(record: &FloatRecord, monitors: &[Monitor], main: Option<PxRect>) -> Option<Placement> {
    let primary = primary(monitors)?;
    let scale = record.pixels_per_point;
    let left = (record.position[0] * scale).round() as i32;
    let top = (record.position[1] * scale).round() as i32;
    let saved = PxRect::from_origin_size(
        left,
        top,
        (record.size[0] * scale).round() as i32,
        (record.size[1] * scale).round() as i32,
    );
    let half = ((TITLE_BAND[0] * scale).min(saved.width() as f32) / 2.0).round() as i32;
    let center = (saved.left + saved.right) / 2;
    let band = PxRect {
        left: center - half,
        top: saved.top,
        right: center + half.max(1),
        bottom: saved.top + (TITLE_BAND[1] * scale).round() as i32,
    };
    let visible = monitors.iter().any(|m| band.overlap_area(&m.work) > 0);
    if !visible {
        return Some(over_main(record.size, monitors, main, primary));
    }
    let monitor = monitors
        .iter()
        .max_by_key(|m| (saved.overlap_area(&m.work), band.overlap_area(&m.work)))
        .unwrap_or(primary);
    let (width, height) = fitted_size(monitor, record.size);
    Some(Placement {
        rect: shifted_into(monitor.work, saved.left, saved.top, width, height),
        scale: monitor.scale,
        maximized: false,
    })
}

/// 主の窓の上の中央（主の窓の中心がある画面の作業領域に収める）。
fn over_main(
    size: [f32; 2],
    monitors: &[Monitor],
    main: Option<PxRect>,
    primary: &Monitor,
) -> Placement {
    let center = main.map(|m| ((m.left + m.right) / 2, (m.top + m.bottom) / 2));
    let monitor = center
        .and_then(|(x, y)| {
            monitors
                .iter()
                .find(|m| m.bounds.overlap_area(&PxRect::from_origin_size(x, y, 1, 1)) > 0)
        })
        .unwrap_or(primary);
    let (width, height) = fitted_size(monitor, size);
    let (cx, cy) = center.unwrap_or((
        (monitor.work.left + monitor.work.right) / 2,
        (monitor.work.top + monitor.work.bottom) / 2,
    ));
    Placement {
        rect: shifted_into(monitor.work, cx - width / 2, cy - height / 2, width, height),
        scale: monitor.scale,
        maximized: false,
    }
}

/// 画素の点がある画面の拡大率（どの画面にも無ければ None）。
pub fn scale_at(monitors: &[Monitor], px: [f32; 2]) -> Option<f32> {
    let at = PxRect::from_origin_size(px[0].round() as i32, px[1].round() as i32, 1, 1);
    monitors
        .iter()
        .find(|m| m.bounds.overlap_area(&at) > 0)
        .map(|m| m.scale)
}
