use super::*;

fn image() -> Snapshot {
    Snapshot {
        origin: [-2, -1],
        size: [2, 2],
        bgra: vec![3, 2, 1, 0, 6, 5, 4, 0, 9, 8, 7, 0, 12, 11, 10, 0],
    }
}

#[test]
fn snapshot_samples_physical_pixels_at_negative_origins_and_edges() {
    let image = image();
    assert_eq!(image.sample([-2, -1]), Some([1, 2, 3]));
    assert_eq!(image.sample([-1, -1]), Some([4, 5, 6]));
    assert_eq!(image.sample([-2, 0]), Some([7, 8, 9]));
    assert_eq!(image.sample([-1, 0]), Some([10, 11, 12]));
    for point in [[-3, 0], [0, 0], [-1, -2], [-1, 1], [i32::MIN, i32::MAX]] {
        assert_eq!(image.sample(point), None);
    }
}

#[test]
fn capture_budget_rejects_invalid_or_oversized_desktops_before_allocation() {
    assert_eq!(Snapshot::byte_len([3840 * 2, 2160]), Ok(66355200));
    assert_eq!(Snapshot::byte_len([0, 20]), Err(Failure::Capture));
    assert_eq!(Snapshot::byte_len([-1, 20]), Err(Failure::Capture));
    assert_eq!(
        Snapshot::byte_len([i32::MAX, i32::MAX]),
        Err(Failure::Budget)
    );
    assert_eq!(Snapshot::byte_len([8192, 4096]), Ok(128 * 1024 * 1024));
    assert_eq!(Snapshot::byte_len([8192, 4097]), Err(Failure::Budget));
}

#[test]
fn loupe_stays_on_the_pointer_monitor_including_negative_coordinates() {
    for bounds in [
        [0, 0, 1920, 1080],
        [-2560, -1440, 0, 0],
        [1920, -400, 4480, 1040],
    ] {
        for point in [
            [bounds[0], bounds[1]],
            [bounds[2] - 1, bounds[3] - 1],
            [bounds[0] + 400, bounds[1] + 200],
        ] {
            let p = loupe_origin(point, bounds, [132, 156]);
            assert!(p[0] >= bounds[0] && p[1] >= bounds[1]);
            assert!(p[0] + 132 <= bounds[2] && p[1] + 156 <= bounds[3]);
            assert!(
                point[0] < p[0]
                    || point[0] >= p[0] + 132
                    || point[1] < p[1]
                    || point[1] >= p[1] + 156
            );
        }
    }
}

struct Fake {
    log: Vec<&'static str>,
    failure: Option<&'static str>,
    event: PickInput,
}
impl Desktop for Fake {
    fn hide(&mut self) {
        self.log.push("hide");
    }
    fn settle(&mut self) -> Result<(), Failure> {
        self.log.push("settle");
        if self.failure == Some("settle") {
            Err(Failure::Capture)
        } else {
            Ok(())
        }
    }
    fn capture(&mut self) -> Result<Snapshot, Failure> {
        self.log.push("capture");
        if self.failure == Some("capture") {
            Err(Failure::Capture)
        } else {
            Ok(image())
        }
    }
    fn select(&mut self, _: &Snapshot) -> Result<Option<[i32; 2]>, Failure> {
        self.log.push("select");
        if self.failure == Some("select") {
            return Err(Failure::Overlay);
        }
        if self.failure == Some("panic") {
            panic!("試験の失敗");
        }
        let mut selection = Selection::default();
        selection.input(std::mem::replace(&mut self.event, PickInput::Cancel));
        Ok(selection.point)
    }
    fn restore(&mut self) {
        self.log.push("restore");
    }
}
fn fake(failure: Option<&'static str>, event: PickInput) -> Fake {
    Fake {
        log: Vec::new(),
        failure,
        event,
    }
}

#[test]
fn hidden_pick_restores_after_success_and_returns_the_captured_rgb() {
    let mut desktop = fake(None, PickInput::Click([-1, 0]));
    assert_eq!(
        session(&mut desktop, Mode::HideWindow),
        Ok(Some([10, 11, 12]))
    );
    assert_eq!(
        desktop.log,
        ["hide", "settle", "capture", "select", "restore"]
    );
    desktop.log.clear();
    desktop.event = PickInput::Click([-2, -1]);
    assert_eq!(session(&mut desktop, Mode::Visible), Ok(Some([1, 2, 3])));
    assert_eq!(desktop.log, ["settle", "capture", "select", "restore"]);
}

#[test]
fn cancellation_cannot_be_overwritten_by_a_later_click() {
    let mut desktop = fake(None, PickInput::Cancel);
    assert_eq!(session(&mut desktop, Mode::HideWindow), Ok(None));
    assert_eq!(
        desktop.log,
        ["hide", "settle", "capture", "select", "restore"]
    );
    let mut selection = Selection::default();
    selection.input(PickInput::Cancel);
    selection.input(PickInput::Click([0, 0]));
    assert!(selection.done);
    assert_eq!(selection.point, None);
}

#[test]
fn every_failure_restores_the_window_including_unwinding() {
    for stage in ["settle", "capture", "select", "panic"] {
        let mut desktop = fake(Some(stage), PickInput::Cancel);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            session(&mut desktop, Mode::HideWindow)
        }));
        assert!(result.is_err() || result.unwrap().is_err());
        assert_eq!(desktop.log.first(), Some(&"hide"));
        assert_eq!(desktop.log.last(), Some(&"restore"));
        assert_eq!(desktop.log.iter().filter(|&&s| s == "restore").count(), 1);
    }
}

#[test]
fn rgb_scalar_and_mask_picking_change_paint_color_without_document_or_undo() {
    let mut app = AppState::new(16, 16);
    let before = app
        .doc
        .composite_pixel(crate::engine::Channel::Color, 0, 0)
        .unwrap();
    let undo = app.can_undo();
    apply_color(&mut app, [255, 0, 128]);
    assert_eq!(app.color.main, [1.0, 0.0, 128.0 / 255.0, 1.0]);
    app.m2.paint_channel = crate::engine::Channel::Roughness;
    apply_color(&mut app, [255, 0, 0]);
    assert_eq!(app.color.main, [0.2126, 0.2126, 0.2126, 1.0]);
    app.m2.paint_channel = crate::engine::Channel::Color;
    app.m2.edit_mask = true;
    apply_color(&mut app, [0, 255, 0]);
    assert_eq!(app.color.main, [0.7152, 0.7152, 0.7152, 1.0]);
    assert_eq!(
        app.doc
            .composite_pixel(crate::engine::Channel::Color, 0, 0)
            .unwrap(),
        before
    );
    assert_eq!(app.can_undo(), undo);
}

#[test]
fn labels_and_failures_have_japanese_and_english_and_linux_has_no_entries() {
    for mode in [Mode::Visible, Mode::HideWindow] {
        assert!(!mode.label(Lang::Ja).is_ascii());
        assert!(mode.label(Lang::En).is_ascii());
    }
    for failure in [
        Failure::Unsupported,
        Failure::Capture,
        Failure::Overlay,
        Failure::Budget,
    ] {
        assert!(!failure.text(Lang::Ja).is_ascii());
        assert!(failure.text(Lang::En).is_ascii());
    }
    if !cfg!(windows) {
        let mut app = AppState::new(16, 16);
        assert!(menu_entries(&app).is_empty());
        request(&mut app, Mode::Visible);
        assert_eq!(app.eyedrop.screen_request, None);
        assert_eq!(app.message, Failure::Unsupported.text(Lang::Ja));
    }
}

struct MessagePointer {
    current: [i32; 2],
    origin: [i32; 2],
}
impl native_input::Pointer for MessagePointer {
    fn cursor_position(&self) -> Option<[i32; 2]> {
        Some(self.current)
    }
    fn client_to_screen(&self, point: [i32; 2]) -> Option<[i32; 2]> {
        Some([point[0] + self.origin[0], point[1] + self.origin[1]])
    }
}
fn packed_point(x: i16, y: i16) -> isize {
    (u32::from(x as u16) | (u32::from(y as u16) << 16)) as isize
}

#[test]
fn queued_click_uses_pressed_pixel_even_after_cursor_moves() {
    let pointer = MessagePointer {
        current: [-1, 0],
        origin: [-2, -1],
    };
    let input = native_input::decode(0x0201, 0, packed_point(0, 0), &pointer).unwrap();
    let mut desktop = fake(None, input);
    assert_eq!(session(&mut desktop, Mode::HideWindow), Ok(Some([1, 2, 3])));
    assert_eq!(
        image().sample(native_input::Pointer::cursor_position(&pointer).unwrap()),
        Some([10, 11, 12])
    );
    assert_eq!(
        desktop.log,
        ["hide", "settle", "capture", "select", "restore"]
    );
}

#[test]
fn click_message_coordinates_are_signed_before_screen_conversion() {
    let pointer = MessagePointer {
        current: [500, 500],
        origin: [-1920, -1080],
    };
    let input = native_input::decode(0x0201, 0, packed_point(-10, -20), &pointer).unwrap();
    let mut selection = Selection::default();
    selection.input(input);
    assert_eq!(selection.point, Some([-1930, -1100]));
}

#[test]
fn escape_key_messages_cancel_and_restore_hidden_window() {
    let pointer = MessagePointer {
        current: [0, 0],
        origin: [0, 0],
    };
    for message in [0x0100, 0x0104] {
        let input = native_input::decode(message, 0x1b, 0, &pointer).expect("Esc は取消入力になる");
        let mut desktop = fake(None, input);
        assert_eq!(session(&mut desktop, Mode::HideWindow), Ok(None));
        assert_eq!(
            desktop.log,
            ["hide", "settle", "capture", "select", "restore"]
        );
        assert!(native_input::decode(message, 0x41, 0, &pointer).is_none());
    }
    assert!(native_input::decode(0x0101, 0x1b, 0, &pointer).is_none());
}
