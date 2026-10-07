//! ウィンドウの置き場所の決め方（Windows）。画面ごとに拡大率が違っても、画面が増減・縮小しても、ウィンドウが見える所へ戻す。
//!
//! 座標の単位は 2 つに分けて持つ。`layout.json` のウィンドウの記録（`WindowRecord`）は「点」で、点 = 画素 / `pixels_per_point`
//! （書いたときのウィンドウがいた画面の拡大率。アプリは egui の拡大を使わないので、egui の点と OS の論理の点が一致する）。
//! 画面の位置は仮想スクリーンの物理画素で、画面ごとに拡大率が違うので、論理の点の座標は画面をまたいで意味を持たない。
//! そこでここでは、記録を画素に直してから、画面（作業領域・拡大率）と突き合わせる。計算は Windows の API に依らない純関数
//! （`plan`・`Settle`・`maximized_client`）で、画面の列挙とウィンドウの手（`native`）だけが Windows のもの。
//!
//! 起動のとき、winit はウィンドウを作った時点の画面（主の画面）の拡大率で論理の位置と大きさを画素に直す。別の拡大率の画面にいたウィンドウは
//! 作った後に移るので、作る時に渡した値では置けない（さらに、別の拡大率の画面へ動かすと OS がウィンドウの大きさを拡大率の比で変える）。
//! eframe は最初の 1 回描くまでウィンドウを隠しているので、最初のフレームから `Settle` が実際のウィンドウの画素を見て、目標とずれていれば
//! 今の拡大率で目標の画素になる値を `ViewportCommand` で頼む（位置→大きさの順。収束するまで数フレーム、上限つき）。
//! egui-winit は `OuterPosition`・`InnerSize` を「点 × 今の拡大率」で画素に直すので、同じ拡大率で割って渡せば画素がそのまま合う。

use egui::{pos2, vec2, ViewportCommand};

use crate::layout::{WindowRecord, MIN_SIZE};

#[cfg(windows)]
mod native;
#[cfg(not(windows))]
mod native {
    use super::Monitor;

    pub fn monitors() -> Vec<Monitor> {
        Vec::new()
    }

    pub fn install(_: &eframe::CreationContext<'_>) {}

    pub fn install_hwnd(_: isize) {}
}

pub use native::install;

/// 初めて起動したときのウィンドウの大きさ（点）。
pub const DEFAULT_SIZE: [f32; 2] = [1600.0, 960.0];
/// ウィンドウの上の帯のうち、つかんで動かせるかを見る部分（点。中央の幅と高さ）。
const TITLE_BAND: [f32; 2] = [200.0, 32.0];
/// ウィンドウの位置・大きさが目標と合っているとみなす差（画素。画素への丸めの差）。
const TOLERANCE: i32 = 2;
/// 置き場所を合わせにいくフレームの上限（これで合わなければ、今の場所のまま使う。利用者が動かしたときに争い続けない）。
const MAX_FRAMES: u32 = 40;

/// 物理画素の矩形（仮想スクリーンの座標。右と下は含まない）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PxRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl PxRect {
    pub fn from_origin_size(left: i32, top: i32, width: i32, height: i32) -> PxRect {
        PxRect {
            left,
            top,
            right: left + width,
            bottom: top + height,
        }
    }

    pub fn width(&self) -> i32 {
        self.right - self.left
    }

    pub fn height(&self) -> i32 {
        self.bottom - self.top
    }

    /// 重なる面積（重ならなければ 0）。
    pub fn overlap_area(&self, other: &PxRect) -> i64 {
        let width = (self.right.min(other.right) - self.left.max(other.left)).max(0);
        let height = (self.bottom.min(other.bottom) - self.top.max(other.top)).max(0);
        i64::from(width) * i64::from(height)
    }
}

/// 画面 1 枚。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Monitor {
    /// 画面全体。
    pub bounds: PxRect,
    /// 作業領域（タスクバーを除いた所。自動で隠すタスクバーは除かれない）。
    pub work: PxRect,
    /// 拡大率（100% が 1.0）。
    pub scale: f32,
    pub primary: bool,
}

/// ウィンドウを置く場所。位置と大きさは物理画素、`scale` は置く画面の拡大率。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub rect: PxRect,
    pub scale: f32,
    pub maximized: bool,
}

impl Placement {
    /// ウィンドウを作るときに渡す内側の大きさ（点。置く画面の拡大率で割った論理の大きさ）。
    pub fn size_points(&self) -> [f32; 2] {
        [
            self.rect.width() as f32 / self.scale,
            self.rect.height() as f32 / self.scale,
        ]
    }
}

/// 記録したウィンドウの位置と大きさ（画素）。
fn record_rect(record: &WindowRecord) -> PxRect {
    let scale = record.pixels_per_point;
    PxRect::from_origin_size(
        (record.position[0] * scale).round() as i32,
        (record.position[1] * scale).round() as i32,
        (record.size[0] * scale).round() as i32,
        (record.size[1] * scale).round() as i32,
    )
}

/// つかんで動かせるかを見る、ウィンドウの上の帯（中央の `TITLE_BAND`）の画素の矩形。
fn title_band(rect: PxRect, scale: f32) -> PxRect {
    let center = (rect.left + rect.right) / 2;
    let half = (TITLE_BAND[0] * scale / 2.0).round() as i32;
    PxRect {
        left: center - half,
        top: rect.top,
        right: center + half,
        bottom: rect.top + (TITLE_BAND[1] * scale).round() as i32,
    }
}

fn primary(monitors: &[Monitor]) -> Option<&Monitor> {
    monitors
        .iter()
        .find(|m| m.primary)
        .or_else(|| monitors.first())
}

/// `size`（点）を `monitor` の拡大率で画素にし、作業領域に収まるまで縮める。ただしウィンドウの最小の大きさ（`MIN_SIZE`）は割らない
/// （作業領域がそれより小さい画面では、OS が最小の大きさを守ってウィンドウが作業領域より大きくなるので、目標もそれに合わせる）。
fn fitted_size(monitor: &Monitor, size: [f32; 2]) -> (i32, i32) {
    let work = monitor.work;
    let fit = |points: f32, limit: i32, minimum: f32| {
        ((points * monitor.scale).round() as i32)
            .min(limit)
            .max((minimum * monitor.scale).round() as i32)
            .max(1)
    };
    (
        fit(size[0], work.width(), MIN_SIZE[0]),
        fit(size[1], work.height(), MIN_SIZE[1]),
    )
}

/// `monitor` の作業領域の中央に、`size`（点。作業領域に収まるまで縮める）のウィンドウを置く。
fn centered(monitor: &Monitor, size: [f32; 2], maximized: bool) -> Placement {
    let work = monitor.work;
    let (width, height) = fitted_size(monitor, size);
    Placement {
        rect: PxRect::from_origin_size(
            work.left + (work.width() - width).max(0) / 2,
            work.top + (work.height() - height).max(0) / 2,
            width,
            height,
        ),
        scale: monitor.scale,
        maximized,
    }
}

/// 起動のときの置き場所。`record` は前に終わったときのウィンドウの記録（無ければ初回）。画面が分からなければ None（OS の置き方のまま）。
/// - 初回: 主の画面の作業領域の中央。大きさは `DEFAULT_SIZE` を、作業領域に収まるまで縮める。
/// - 記録あり: 上の帯の中央が今のどれかの画面の作業領域に見えていれば、その画面（ウィンドウと一番重なる画面）に置く。大きさは記録の点を
///   その画面の拡大率で直し、作業領域に収まるまで縮め、位置はウィンドウ全体が作業領域に入るよう寄せる。
///   帯がどの画面にも見えなければ（画面が無くなった・解像度や配置が変わった）、初回と同じく主の画面の中央に置く（大きさは記録の点）。
pub fn plan(record: Option<&WindowRecord>, monitors: &[Monitor]) -> Option<Placement> {
    let primary = primary(monitors)?;
    let Some(record) = record else {
        return Some(centered(primary, DEFAULT_SIZE, false));
    };
    let saved = record_rect(record);
    let band = title_band(saved, record.pixels_per_point);
    let visible = monitors.iter().any(|m| band.overlap_area(&m.work) > 0);
    if !visible {
        return Some(centered(primary, record.size, record.maximized));
    }
    let monitor = monitors
        .iter()
        .max_by_key(|m| (saved.overlap_area(&m.work), band.overlap_area(&m.work)))
        .unwrap_or(primary);
    let work = monitor.work;
    let (width, height) = fitted_size(monitor, record.size);
    // （`clamp` は最小が最大を超えると落ちるので、`min`・`max` で寄せる。ウィンドウが作業領域より大きいときは、左上を作業領域の左上に合わせる）
    let left = saved.left.min(work.right - width).max(work.left);
    let top = saved.top.min(work.bottom - height).max(work.top);
    Some(Placement {
        rect: PxRect::from_origin_size(left, top, width, height),
        scale: monitor.scale,
        maximized: record.maximized,
    })
}

/// ウィンドウの今の様子（毎フレーム、egui が渡すウィンドウの情報から作る）。画素は `scale`（ウィンドウの今の拡大率）を掛けた物理画素。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Actual {
    /// 外枠の左上（画素）。
    pub position: [i32; 2],
    /// 内側の大きさ（画素）。
    pub size: [i32; 2],
    /// ウィンドウの今の拡大率。
    pub scale: f32,
}

impl Actual {
    /// egui のウィンドウの情報（外枠と内側の矩形は点、`native_pixels_per_point` は OS の拡大率）から。まだ分からなければ None。
    pub fn from_viewport(info: &egui::ViewportInfo) -> Option<Actual> {
        let scale = info.native_pixels_per_point?;
        let outer = info.outer_rect?;
        let inner = info.inner_rect?;
        Some(Actual {
            position: [
                (outer.min.x * scale).round() as i32,
                (outer.min.y * scale).round() as i32,
            ],
            size: [
                (inner.width() * scale).round() as i32,
                (inner.height() * scale).round() as i32,
            ],
            scale,
        })
    }
}

/// 起動の置き場所を、実際のウィンドウに合うまで 1 フレームずつ頼む状態。
#[derive(Debug)]
pub struct Settle {
    target: Placement,
    frames: u32,
}

/// `Settle::step` の結果。
#[derive(Debug, PartialEq)]
pub struct Step {
    /// ウィンドウへ頼むこと。
    pub commands: Vec<ViewportCommand>,
    /// まだ合わせている途中か（true の間は次のフレームも呼ぶ）。
    pub settling: bool,
}

impl Settle {
    pub fn new(target: Placement) -> Settle {
        Settle { target, frames: 0 }
    }

    /// 1 フレーム。`actual` が目標と合っていれば（最大化する記録なら最大化を頼んで）終わる。
    /// 拡大率の違う画面へ動かすと OS がウィンドウの大きさを変えるので、ウィンドウの拡大率が目標の画面と違う間は位置だけを頼み、
    /// 画面が合ってから大きさを頼む。ウィンドウの情報がまだ無い間は待つ。上限のフレームで合わなければ、今のまま終わる。
    pub fn step(&mut self, actual: Option<Actual>) -> Step {
        self.frames += 1;
        let done = |target: &Placement| Step {
            commands: if target.maximized {
                vec![ViewportCommand::Maximized(true)]
            } else {
                Vec::new()
            },
            settling: false,
        };
        if self.frames > MAX_FRAMES {
            return done(&self.target);
        }
        let Some(actual) = actual else {
            return Step {
                commands: Vec::new(),
                settling: true,
            };
        };
        let want = self.target.rect;
        let moved = (actual.position[0] - want.left).abs() > TOLERANCE
            || (actual.position[1] - want.top).abs() > TOLERANCE;
        let resized = (actual.size[0] - want.width()).abs() > TOLERANCE
            || (actual.size[1] - want.height()).abs() > TOLERANCE;
        if !moved && !resized {
            return done(&self.target);
        }
        let scale = actual.scale;
        let same_screen = (scale - self.target.scale).abs() < 0.01;
        let mut commands = Vec::new();
        if moved {
            commands.push(ViewportCommand::OuterPosition(pos2(
                want.left as f32 / scale,
                want.top as f32 / scale,
            )));
        }
        if resized && (same_screen || !moved) {
            commands.push(ViewportCommand::InnerSize(vec2(
                want.width() as f32 / scale,
                want.height() as f32 / scale,
            )));
        }
        Step {
            commands,
            settling: true,
        }
    }
}

/// 自動で隠すタスクバーがある辺（最大化したウィンドウの縁で、タスクバーを呼び出せるように 1 画素だけ空ける辺）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

impl Edges {
    pub fn any(&self) -> bool {
        self.left || self.top || self.right || self.bottom
    }
}

/// 枠を外したウィンドウを最大化したときの内側（`work` は OS が最大化に使う作業領域）。自動で隠すタスクバーのある辺は、画面の端いっぱいまで
/// 覆うとマウスを端へ寄せてもタスクバーが出てこないので、その辺だけ 1 画素縮める（Windows の枠つきのウィンドウの最大化も、同じ 1 画素を残す）。
pub fn maximized_client(work: PxRect, autohide: Edges) -> PxRect {
    let mut rect = work;
    // ウィンドウが 3 画素より小さければ縮めない（壊れた値で矩形が裏返らないように）
    if rect.width() > 2 && rect.height() > 2 {
        rect.left += i32::from(autohide.left);
        rect.top += i32::from(autohide.top);
        rect.right -= i32::from(autohide.right);
        rect.bottom -= i32::from(autohide.bottom);
    }
    rect
}

// ───────── 起動の手続き（実際のウィンドウだけ） ─────────

use std::sync::Mutex;

static SETTLING: Mutex<Option<Settle>> = Mutex::new(None);

/// 起動のときの置き場所を決める（画面を列挙できる OS だけ。できなければ None で、ウィンドウは OS の置き方のまま）。
/// 決めたら `remember` で覚え、ウィンドウを作るときの大きさは `Placement::size_points` を使う。
pub fn startup(record: Option<&WindowRecord>) -> Option<Placement> {
    plan(record, &native::monitors())
}

/// 起動の置き場所を、最初のフレームから `settle` が合わせにいくように覚える。
pub fn remember(target: Placement) {
    *SETTLING.lock().unwrap_or_else(|e| e.into_inner()) = Some(Settle::new(target));
}

/// 毎フレーム呼ぶ。起動の置き場所を合わせている間は、頼みを送って true を返す（その間はウィンドウの位置を記録・保存しない）。
pub fn settle(ctx: &egui::Context) -> bool {
    let mut guard = SETTLING.lock().unwrap_or_else(|e| e.into_inner());
    let Some(settling) = guard.as_mut() else {
        return false;
    };
    let info = ctx.input(|i| i.viewport().clone());
    // 最小化中・全画面中はウィンドウの情報が当てにならないので待つ（隠したまま起動する場合など）
    let actual = if info.minimized == Some(true) || info.fullscreen == Some(true) {
        None
    } else {
        Actual::from_viewport(&info)
    };
    let step = settling.step(actual);
    for command in step.commands {
        ctx.send_viewport_cmd(command);
    }
    if step.settling {
        ctx.request_repaint();
    } else {
        *guard = None;
    }
    step.settling
}

/// 今つながっている画面（画面を列挙できる OS だけ。できなければ空）。
pub fn monitors() -> Vec<Monitor> {
    native::monitors()
}

/// 別ウィンドウ（ウィンドウのハンドル。Windows の HWND の値）の最大化を、メインウィンドウと同じく自動で隠すタスクバーに合わせる（Windows だけ。ほかは何もしない）。
pub fn install_hwnd(hwnd: isize) {
    native::install_hwnd(hwnd);
}
