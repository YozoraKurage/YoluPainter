//! デスクトップの静止画から色を取る。画像はセッション内だけで所有し、保存・送信しない。
//! Windows は GDI の SDR 出力を sRGB 8 bit として扱う（HDR/ICC の測色は保証しない）。

use crate::{
    engine::ChannelKind,
    lang::Lang,
    state::{Action, AppState},
    ui::{menu::Entry, theme as t, widgets as w},
};
use egui::{pos2, vec2, Key, Modifiers, Rect, Ui};

#[cfg(any(windows, test))]
mod native_input;
#[cfg(test)]
mod tests;
#[cfg(windows)]
mod windows;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Visible,
    HideWindow,
}
impl Mode {
    pub fn label(self, lang: Lang) -> &'static str {
        match self {
            Self::Visible => lang.pick("画面の色を取得", "Pick Screen Color"),
            Self::HideWindow => lang.pick(
                "ウィンドウを隠して画面の色を取得",
                "Hide Window and Pick Screen Color",
            ),
        }
    }
    fn shortcut(self) -> &'static str {
        match self {
            Self::Visible => "Ctrl+Alt+I",
            Self::HideWindow => "Ctrl+Alt+Shift+I",
        }
    }
}

#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    Unsupported,
    Capture,
    Overlay,
    Budget,
}
impl Failure {
    fn text(self, lang: Lang) -> &'static str {
        match self {
            Self::Unsupported => lang.pick(
                "画面の色の取得は Windows のみ対応",
                "Screen color picking is available on Windows only",
            ),
            Self::Capture => lang.pick("画面を取得できません", "Screen capture unavailable"),
            Self::Overlay => lang.pick(
                "画面の色の選択を開始できません",
                "Screen color picker unavailable",
            ),
            Self::Budget => lang.pick(
                "画面の大きさが取得の上限を超えています",
                "Screen capture size limit exceeded",
            ),
        }
    }
}

pub fn menu_entries(app: &AppState) -> Vec<Entry<Action>> {
    if !cfg!(windows) {
        return Vec::new();
    }
    [Mode::Visible, Mode::HideWindow]
        .into_iter()
        .map(|mode| {
            Entry::item(mode.label(app.lang), Action::ScreenPick(mode))
                .shortcut(mode.shortcut())
                .enabled(!app.is_stroking())
        })
        .collect()
}

pub fn request(app: &mut AppState, mode: Mode) {
    if !cfg!(windows) {
        app.message = Failure::Unsupported.text(app.lang).into();
    } else if !app.is_stroking() {
        app.eyedrop.screen_request = Some(mode);
    }
}

pub fn shortcuts(ctx: &egui::Context, app: &mut AppState) {
    if !cfg!(windows) || app.is_stroking() {
        return;
    }
    for mode in [Mode::HideWindow, Mode::Visible] {
        let modifiers = Modifiers::CTRL
            | Modifiers::ALT
            | if mode == Mode::HideWindow {
                Modifiers::SHIFT
            } else {
                Modifiers::NONE
            };
        if ctx.input_mut(|i| i.consume_key(modifiers, Key::I)) {
            request(app, mode);
            break;
        }
    }
}

pub fn options(ui: &mut Ui, app: &mut AppState, r: Rect, mut x: f32) {
    if !cfg!(windows) {
        return;
    }
    for mode in [Mode::Visible, Mode::HideWindow] {
        let label = mode.label(app.lang);
        let width = w::text_width(ui.painter(), label, t::LABEL) + 18.0;
        if x + width > r.right() - 8.0 {
            break;
        }
        if w::button(
            ui,
            Rect::from_min_size(pos2(x, r.top() + 6.0), vec2(width, r.height() - 12.0)),
            mode.shortcut(),
            label,
            false,
            !app.is_stroking(),
            Some(mode.shortcut()),
            None,
        )
        .clicked()
        {
            request(app, mode);
        }
        x += width + 6.0;
    }
}

pub fn frame(app: &mut AppState, frame: &eframe::Frame) {
    let Some(mode) = app.eyedrop.screen_request.take() else {
        return;
    };
    if app.is_stroking() {
        return;
    }
    #[cfg(windows)]
    let result = windows::pick(frame, mode);
    #[cfg(not(windows))]
    let result = {
        let _ = (frame, mode);
        Err(Failure::Unsupported)
    };
    match result {
        Ok(Some(rgb)) => apply_color(app, rgb),
        Ok(None) => {}
        Err(error) => app.message = error.text(app.lang).into(),
    }
}

fn apply_color(app: &mut AppState, rgb: [u8; 3]) {
    let scalar = app.m2.edit_mask
        || app
            .doc
            .channel_info(app.m2.paint_channel)
            .is_some_and(|i| i.kind == ChannelKind::Scalar);
    let mut values = rgb.map(|v| v as f32 / 255.0);
    if scalar {
        // sRGB の値で重みづけした輝度（文書のスカラーも保存した値のまま扱う）。
        let y = values[0] * 0.2126 + values[1] * 0.7152 + values[2] * 0.0722;
        values = [y; 3];
    }
    app.color.set_main([values[0], values[1], values[2], 1.0]);
    if app.paints_material() {
        match app.m2.paint_channel {
            crate::engine::Channel::Emission => app.mat.emission = values,
            channel if scalar => app.mat.set_scalar(channel, values[0]),
            _ => {}
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Debug)]
struct Snapshot {
    origin: [i32; 2],
    size: [i32; 2],
    bgra: Vec<u8>,
}
#[cfg(any(windows, test))]
impl Snapshot {
    fn byte_len(size: [i32; 2]) -> Result<usize, Failure> {
        if size[0] <= 0 || size[1] <= 0 {
            return Err(Failure::Capture);
        }
        let len = (size[0] as usize)
            .checked_mul(size[1] as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or(Failure::Budget)?;
        // DIB と CPU の写しを合わせて最大 256 MiB。
        if len > 128 * 1024 * 1024 {
            return Err(Failure::Budget);
        }
        Ok(len)
    }
    fn sample(&self, point: [i32; 2]) -> Option<[u8; 3]> {
        let x = i64::from(point[0]) - i64::from(self.origin[0]);
        let y = i64::from(point[1]) - i64::from(self.origin[1]);
        if x < 0 || y < 0 || x >= i64::from(self.size[0]) || y >= i64::from(self.size[1]) {
            return None;
        }
        let at = (y as usize * self.size[0] as usize + x as usize) * 4;
        let px = self.bgra.get(at..at + 4)?;
        Some([px[2], px[1], px[0]])
    }
}

/// ルーペは今いるモニターの内側に収める（負座標の画面と端でも同じ）。
#[cfg(any(windows, test))]
fn loupe_origin(point: [i32; 2], bounds: [i32; 4], size: [i32; 2]) -> [i32; 2] {
    std::array::from_fn(|axis| {
        let low = bounds[axis];
        let high = (bounds[axis + 2] - size[axis]).max(low);
        let preferred = if point[axis] + 24 + size[axis] <= bounds[axis + 2] {
            point[axis] + 24
        } else {
            point[axis] - 24 - size[axis]
        };
        preferred.clamp(low, high)
    })
}

#[cfg(any(windows, test))]
trait Desktop {
    fn hide(&mut self);
    fn settle(&mut self) -> Result<(), Failure>;
    fn capture(&mut self) -> Result<Snapshot, Failure>;
    fn select(&mut self, image: &Snapshot) -> Result<Option<[i32; 2]>, Failure>;
    fn restore(&mut self);
}
#[cfg(any(windows, test))]
fn session(desktop: &mut impl Desktop, mode: Mode) -> Result<Option<[u8; 3]>, Failure> {
    struct Restore<'a, D: Desktop>(&'a mut D);
    impl<D: Desktop> Drop for Restore<'_, D> {
        fn drop(&mut self) {
            self.0.restore();
        }
    }
    let guard = Restore(desktop);
    if mode == Mode::HideWindow {
        guard.0.hide();
    }
    guard.0.settle()?;
    let image = guard.0.capture()?;
    let selected = guard.0.select(&image)?;
    Ok(selected.and_then(|point| image.sample(point)))
}

#[cfg(any(windows, test))]
enum PickInput {
    Click([i32; 2]),
    Cancel,
}
#[cfg(any(windows, test))]
#[derive(Clone, Copy, Default)]
struct Selection {
    done: bool,
    point: Option<[i32; 2]>,
}
#[cfg(any(windows, test))]
impl Selection {
    fn input(&mut self, event: PickInput) {
        if self.done {
            return;
        }
        self.point = match event {
            PickInput::Click(point) => Some(point),
            PickInput::Cancel => None,
        };
        self.done = true;
    }
}
