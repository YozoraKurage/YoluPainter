//! ファイルを落とした場所（ウィンドウの中の egui の点）。
//!
//! winit 0.30 は、Windows と macOS のファイルのドロップで、落とした点を渡さない（`DroppedFile`・`HoveredFile` はファイルのパスだけで、点を持たない）。
//! ほかのアプリからドラッグしてくると、カーソルは一度ウィンドウの外へ出るので、egui のポインタの位置（`latest_pos`）も無い。
//! そこで、egui の位置があればそれを使い、無いときだけ OS のカーソルの位置をそのビューポートの egui の点に直して使う。
//! 落としていない間（`dropped_files` も `hovered_files` も空）は OS を呼ばない。
//! Linux は OS の位置を持たず、egui の位置だけ。座標の計算は OS を呼ばない純粋な関数に分けてある。

use egui::{Pos2, Rect};

/// ファイルを落とした場所（落としている間でなければ、egui のポインタの位置）。
pub fn drop_point(ctx: &egui::Context) -> Option<Pos2> {
    let (egui_pos, dropping) = ctx.input(|i| {
        (
            i.pointer.latest_pos(),
            !i.raw.dropped_files.is_empty() || !i.raw.hovered_files.is_empty(),
        )
    });
    choose(egui_pos, dropping, || os_point(ctx))
}

/// egui の位置が先。無く、ファイルを落としている間だけ OS の位置を取りに行く。
fn choose(
    egui_pos: Option<Pos2>,
    dropping: bool,
    os: impl FnOnce() -> Option<Pos2>,
) -> Option<Pos2> {
    match egui_pos {
        Some(p) => Some(p),
        None if dropping => os(),
        None => None,
    }
}

/// Windows のカーソル（画面の物理の画素）を、ウィンドウの内側の左上を原点にした egui の点にする。
/// egui-winit の `inner_rect`（内側の位置の画素 ÷ pixels_per_point）と同じ単位。内側の矩形が分からなければ None。
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_point(cursor_px: [i32; 2], pixels_per_point: f32, inner: Option<Rect>) -> Option<Pos2> {
    let inner = inner?;
    if !(pixels_per_point.is_finite() && pixels_per_point > 0.0) {
        return None;
    }
    Some(
        Pos2::new(
            cursor_px[0] as f32 / pixels_per_point,
            cursor_px[1] as f32 / pixels_per_point,
        ) - inner.min.to_vec2(),
    )
}

/// macOS の contentView の点（原点は反転していなければ左下）を、上が原点の egui の点にする。
/// egui の点 = ビューの点 ÷ 拡大率（`zoom_factor`）。
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn mac_point(view_point: [f64; 2], height: f64, flipped: bool, zoom: f32) -> Option<Pos2> {
    if !(view_point[0].is_finite() && view_point[1].is_finite() && zoom.is_finite() && zoom > 0.0) {
        return None;
    }
    let y = if flipped {
        view_point[1]
    } else {
        height - view_point[1]
    };
    Some(Pos2::new(view_point[0] as f32 / zoom, y as f32 / zoom))
}

#[cfg(windows)]
fn os_point(ctx: &egui::Context) -> Option<Pos2> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    let mut at = POINT::default();
    // SAFETY: 出力の POINT への参照だけを渡す。
    unsafe { GetCursorPos(&mut at) }.ok()?;
    windows_point(
        [at.x, at.y],
        ctx.pixels_per_point(),
        ctx.input(|i| i.viewport().inner_rect),
    )
}

#[cfg(target_os = "macos")]
fn os_point(ctx: &egui::Context) -> Option<Pos2> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSWindow};

    let mtm = MainThreadMarker::new()?;
    let at = NSEvent::mouseLocation();
    // カーソルの下のウィンドウ（ほかのアプリのウィンドウなら、このアプリの表に無いので None）
    let number = NSWindow::windowNumberAtPoint_belowWindowWithWindowNumber(at, 0, mtm);
    let window = NSApplication::sharedApplication(mtm).windowWithWindowNumber(number)?;
    let view = window.contentView()?;
    let local = view.convertPoint_fromView(window.convertPointFromScreen(at), None);
    mac_point(
        [local.x, local.y],
        view.bounds().size.height,
        view.isFlipped(),
        ctx.zoom_factor(),
    )
}

#[cfg(not(any(windows, target_os = "macos")))]
fn os_point(_ctx: &egui::Context) -> Option<Pos2> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{pos2, vec2};

    fn rect(x: f32, y: f32) -> Option<Rect> {
        Some(Rect::from_min_size(pos2(x, y), vec2(800.0, 600.0)))
    }

    #[test]
    fn a_windows_cursor_is_divided_by_the_scale_and_made_relative_to_the_inner_corner() {
        // 内側の左上が画素 (100, 50) で、拡大率 1.0 なら点も同じ数
        assert_eq!(
            windows_point([400, 300], 1.0, rect(100.0, 50.0)),
            Some(pos2(300.0, 250.0))
        );
        // 拡大率 1.5: 画素 (450, 300) は点 (300, 200)。内側の左上は点 (100, 40)（画素 150, 60）
        assert_eq!(
            windows_point([450, 300], 1.5, rect(100.0, 40.0)),
            Some(pos2(200.0, 160.0))
        );
    }

    #[test]
    fn a_windows_cursor_works_when_the_window_is_on_a_monitor_left_of_the_primary() {
        // ウィンドウの内側の左上が画素 (-1920, 100)・拡大率 1.0（点も -1920, 100）。カーソルは画素 (-1820, 150)
        assert_eq!(
            windows_point([-1820, 150], 1.0, rect(-1920.0, 100.0)),
            Some(pos2(100.0, 50.0))
        );
        // 拡大率 1.5 では、内側の左上は点 (-1280, 66.666...)。カーソル画素 (-1820, 250) は点 (-1213.33, 166.66)
        let p = windows_point([-1820, 250], 1.5, rect(-1280.0, 100.0 / 1.5)).unwrap();
        assert!((p.x - 66.666_67).abs() < 1e-3 && (p.y - 100.0).abs() < 1e-3);
    }

    #[test]
    fn a_windows_cursor_without_the_inner_rect_or_a_usable_scale_is_none() {
        assert_eq!(windows_point([10, 10], 1.0, None), None);
        assert_eq!(windows_point([10, 10], 0.0, rect(0.0, 0.0)), None);
        assert_eq!(windows_point([10, 10], f32::NAN, rect(0.0, 0.0)), None);
    }

    #[test]
    fn a_mac_view_point_is_flipped_to_the_top_origin_and_divided_by_the_zoom() {
        // 反転していないビュー（原点は左下）: 高さ 600 で下から 100 は、上から 500
        assert_eq!(
            mac_point([200.0, 100.0], 600.0, false, 1.0),
            Some(pos2(200.0, 500.0))
        );
        // 反転したビュー（原点は左上）: そのまま
        assert_eq!(
            mac_point([200.0, 100.0], 600.0, true, 1.0),
            Some(pos2(200.0, 100.0))
        );
        // 拡大率 1.25
        assert_eq!(
            mac_point([250.0, 100.0], 600.0, true, 1.25),
            Some(pos2(200.0, 80.0))
        );
        assert_eq!(
            mac_point([250.0, 100.0], 600.0, false, 1.25),
            Some(pos2(200.0, 400.0))
        );
    }

    #[test]
    fn a_mac_view_point_that_is_not_a_number_is_none() {
        assert_eq!(mac_point([f64::NAN, 1.0], 600.0, false, 1.0), None);
        assert_eq!(mac_point([1.0, 1.0], 600.0, false, 0.0), None);
    }

    #[test]
    fn the_egui_position_comes_first_and_the_os_is_asked_only_when_it_is_missing_while_dropping() {
        let from_egui = Some(pos2(1.0, 2.0));
        let from_os = Some(pos2(3.0, 4.0));
        // egui の位置があれば、落としている間でも OS を呼ばない
        assert_eq!(
            choose(from_egui, true, || panic!("OS を呼んではいけない")),
            from_egui
        );
        // egui が None で落としている間だけ OS の位置
        assert_eq!(choose(None, true, || from_os), from_os);
        assert_eq!(choose(None, true, || None), None);
        // 落としていなければ、egui が None でも OS を呼ばない
        assert_eq!(
            choose(None, false, || panic!("OS を呼んではいけない")),
            None
        );
    }

    #[test]
    fn without_a_drop_the_point_is_the_egui_pointer_position() {
        let ctx = egui::Context::default();
        let mut raw = egui::RawInput::default();
        raw.events.push(egui::Event::PointerMoved(pos2(10.0, 20.0)));
        let mut output = ctx.run_ui(raw, |_| {});
        output.textures_delta.clear();
        assert_eq!(drop_point(&ctx), Some(pos2(10.0, 20.0)));
    }
}
