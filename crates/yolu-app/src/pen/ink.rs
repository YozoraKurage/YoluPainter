//! Windows Ink（`POINTER_PEN_INFO`）の履歴の点 1 つを、ペンの 1 点の値に直す（OS に依らない純粋な部分）。
//! `GetPointerPenInfoHistory` などの呼び出しとクライアントの位置への変換（`ScreenToClient`）は `win_ink`（Windows だけ）。ここは Linux でも単体テストできる
//! （`wintab` と `win_tab` の分け方と同じ）。
//!
//! 値の直し方:
//! - 位置: 機器の矩形と画面の矩形が読めて、機器の矩形に広さがあれば、himetric の位置を画面の矩形へ f64 で写す（整数の画素より細かい）。
//!   そうでなければ `ptPixelLocation`（整数の画素）。
//! - 筆圧: `PEN_MASK_PRESSURE` があれば `pressure / 1024` を 0〜1 に丸める。無ければ 1。
//! - 傾き: `PEN_MASK_TILT_X`・`PEN_MASK_TILT_Y` ごとに、あればその値（度）、無ければ 0。
//! - 回転: `PEN_MASK_ROTATION` があれば `rotation % 360`、無ければ None。
//! - 触れている: `POINTER_FLAG_INCONTACT`。消しゴムの端: `PEN_FLAG_ERASER` か `PEN_FLAG_INVERTED`。サイドボタン: `PEN_FLAG_BARREL`。
//! - 時刻: `dwTime`。

use super::PenSample;
use crate::engine::Tilt;

// ───────── 定数（winuser.h・winuser の POINTER_FLAG_*。`win_ink` が `windows` クレートの値と同じことをビルドのときに確かめる） ─────────

pub const PEN_FLAG_BARREL: u32 = 0x0000_0001;
pub const PEN_FLAG_INVERTED: u32 = 0x0000_0002;
pub const PEN_FLAG_ERASER: u32 = 0x0000_0004;
pub const PEN_MASK_PRESSURE: u32 = 0x0000_0001;
pub const PEN_MASK_ROTATION: u32 = 0x0000_0002;
pub const PEN_MASK_TILT_X: u32 = 0x0000_0004;
pub const PEN_MASK_TILT_Y: u32 = 0x0000_0008;
pub const POINTER_FLAG_INCONTACT: u32 = 0x0000_0004;

/// 矩形（`RECT` と同じ並び）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InkRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// 履歴の点 1 つの生の値（`POINTER_PEN_INFO` と `POINTER_INFO` から、`win_ink` が写す）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InkPoint {
    /// `ptHimetricLocation`。
    pub himetric: [i32; 2],
    /// `ptPixelLocation`（画面の整数の画素）。
    pub pixel: [i32; 2],
    /// `GetPointerDeviceRects` が成功したときの（機器の矩形、画面の矩形）。失敗したときは None。
    pub rects: Option<(InkRect, InkRect)>,
    /// `penMask`。
    pub pen_mask: u32,
    /// `pressure`（0〜1024）。
    pub pressure: u32,
    pub tilt_x: i32,
    pub tilt_y: i32,
    /// `rotation`（度）。
    pub rotation: u32,
    /// `pointerInfo.pointerFlags`。
    pub pointer_flags: u32,
    /// `penFlags`。
    pub pen_flags: u32,
    /// メッセージのポインタの番号。
    pub pointer_id: u32,
    /// `pointerInfo.dwTime`。
    pub time_ms: u32,
}

/// `PenSample` のうち位置以外の値（位置はクライアントの位置に直してから `with_pos` で付ける）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InkValues {
    pub pressure: f32,
    pub tilt: Tilt,
    pub rotation: Option<f32>,
    pub contact: bool,
    pub eraser: bool,
    pub barrel: bool,
    pub pointer_id: u32,
    pub time_ms: u32,
}

impl InkValues {
    /// クライアントの位置 `pos` を付けて `PenSample` にする。
    pub fn with_pos(self, pos: [f32; 2]) -> PenSample {
        PenSample {
            pos,
            pressure: self.pressure,
            tilt: self.tilt,
            rotation: self.rotation,
            contact: self.contact,
            eraser: self.eraser,
            barrel: self.barrel,
            pointer_id: self.pointer_id,
            time_ms: self.time_ms,
        }
    }
}

/// 点の画面の位置（仮想スクリーンの物理の画素、f64）。
pub fn screen_position(p: &InkPoint) -> [f64; 2] {
    if let Some((device, display)) = p.rects {
        if device.right > device.left && device.bottom > device.top {
            let rx = (display.right - display.left) as f64 / (device.right - device.left) as f64;
            let ry = (display.bottom - display.top) as f64 / (device.bottom - device.top) as f64;
            return [
                display.left as f64 + p.himetric[0] as f64 * rx,
                display.top as f64 + p.himetric[1] as f64 * ry,
            ];
        }
    }
    [p.pixel[0] as f64, p.pixel[1] as f64]
}

/// 履歴の点 1 つを、位置以外の値と画面の位置にする。
pub fn read(p: &InkPoint) -> (InkValues, [f64; 2]) {
    let pressure = if p.pen_mask & PEN_MASK_PRESSURE != 0 {
        (p.pressure as f32 / 1024.0).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let tilt = Tilt {
        x: if p.pen_mask & PEN_MASK_TILT_X != 0 {
            p.tilt_x as f32
        } else {
            0.0
        },
        y: if p.pen_mask & PEN_MASK_TILT_Y != 0 {
            p.tilt_y as f32
        } else {
            0.0
        },
    };
    let values = InkValues {
        pressure,
        tilt,
        rotation: (p.pen_mask & PEN_MASK_ROTATION != 0).then_some((p.rotation % 360) as f32),
        contact: p.pointer_flags & POINTER_FLAG_INCONTACT != 0,
        eraser: p.pen_flags & (PEN_FLAG_ERASER | PEN_FLAG_INVERTED) != 0,
        barrel: p.pen_flags & PEN_FLAG_BARREL != 0,
        pointer_id: p.pointer_id,
        time_ms: p.time_ms,
    };
    (values, screen_position(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECT_1024: InkRect = InkRect {
        left: 0,
        top: 0,
        right: 1024,
        bottom: 1024,
    };

    /// 筆圧のマスクがある点（既定の他の値はすべて 0）。
    fn with_pressure(raw: u32) -> InkPoint {
        InkPoint {
            pen_mask: PEN_MASK_PRESSURE,
            pressure: raw,
            ..InkPoint::default()
        }
    }

    #[test]
    fn the_pressure_is_the_raw_value_over_1024_clamped_to_0_1() {
        assert_eq!(read(&with_pressure(0)).0.pressure, 0.0);
        assert_eq!(read(&with_pressure(256)).0.pressure, 0.25);
        assert_eq!(read(&with_pressure(512)).0.pressure, 0.5);
        assert_eq!(read(&with_pressure(1024)).0.pressure, 1.0);
        assert_eq!(read(&with_pressure(2048)).0.pressure, 1.0);
        assert_eq!(read(&with_pressure(u32::MAX)).0.pressure, 1.0);
    }

    #[test]
    fn a_pen_without_the_pressure_mask_has_pressure_1_whatever_the_raw_value() {
        for raw in [0, 300, 1024] {
            let p = InkPoint {
                pen_mask: PEN_MASK_TILT_X | PEN_MASK_ROTATION,
                pressure: raw,
                ..InkPoint::default()
            };
            assert_eq!(read(&p).0.pressure, 1.0);
        }
    }

    #[test]
    fn each_tilt_axis_is_read_only_with_its_own_mask() {
        let base = InkPoint {
            tilt_x: -45,
            tilt_y: 30,
            ..InkPoint::default()
        };
        let tilt = |mask| {
            let t = read(&InkPoint {
                pen_mask: mask,
                ..base
            })
            .0
            .tilt;
            (t.x, t.y)
        };
        assert_eq!(tilt(0), (0.0, 0.0));
        assert_eq!(tilt(PEN_MASK_TILT_X), (-45.0, 0.0));
        assert_eq!(tilt(PEN_MASK_TILT_Y), (0.0, 30.0));
        assert_eq!(tilt(PEN_MASK_TILT_X | PEN_MASK_TILT_Y), (-45.0, 30.0));
        // 筆圧・回転のマスクは傾きに効かない
        assert_eq!(tilt(PEN_MASK_PRESSURE | PEN_MASK_ROTATION), (0.0, 0.0));
    }

    #[test]
    fn the_rotation_is_none_without_its_mask_and_wraps_at_360() {
        let rotation = |mask, raw| {
            read(&InkPoint {
                pen_mask: mask,
                rotation: raw,
                ..InkPoint::default()
            })
            .0
            .rotation
        };
        assert_eq!(rotation(0, 90), None);
        assert_eq!(rotation(PEN_MASK_PRESSURE | PEN_MASK_TILT_X, 90), None);
        // マスクがあれば 0° も Some(0.0)（回転を送れないペンの None とは別）
        assert_eq!(rotation(PEN_MASK_ROTATION, 0), Some(0.0));
        assert_eq!(rotation(PEN_MASK_ROTATION, 90), Some(90.0));
        assert_eq!(rotation(PEN_MASK_ROTATION, 359), Some(359.0));
        assert_eq!(rotation(PEN_MASK_ROTATION, 360), Some(0.0));
        assert_eq!(rotation(PEN_MASK_ROTATION, 725), Some(5.0));
    }

    #[test]
    fn contact_follows_the_incontact_pointer_flag_only() {
        let contact = |flags| {
            read(&InkPoint {
                pointer_flags: flags,
                ..InkPoint::default()
            })
            .0
            .contact
        };
        assert!(!contact(0));
        assert!(contact(0x4));
        // NEW（1）・INRANGE（2）だけでは触れていない
        assert!(!contact(0x1 | 0x2));
        // INRANGE・INCONTACT・UPDATE（0x20000）
        assert!(contact(0x2 | 0x4 | 0x2_0000));
        // UP（0x40000）の点は、INCONTACT が無ければ触れていない
        assert!(!contact(0x2 | 0x4_0000));
        // ペンのフラグのビットは接触に効かない
        let p = InkPoint {
            pen_flags: 0x7,
            ..InkPoint::default()
        };
        assert!(!read(&p).0.contact);
    }

    #[test]
    fn the_eraser_end_is_the_eraser_or_the_inverted_flag_and_the_barrel_is_its_own_flag() {
        let flags = |pen_flags| {
            let v = read(&InkPoint {
                pen_flags,
                ..InkPoint::default()
            })
            .0;
            (v.eraser, v.barrel)
        };
        assert_eq!(flags(0), (false, false));
        assert_eq!(flags(0x1), (false, true));
        assert_eq!(flags(0x2), (true, false), "INVERTED");
        assert_eq!(flags(0x4), (true, false), "ERASER");
        assert_eq!(flags(0x6), (true, false), "ERASER と INVERTED");
        assert_eq!(flags(0x3), (true, true), "INVERTED とサイドボタン");
        assert_eq!(flags(0x5), (true, true), "ERASER とサイドボタン");
        // 触れているかは別（接触のフラグは消しゴムの端を決めない）
        let p = InkPoint {
            pointer_flags: 0x4,
            ..InkPoint::default()
        };
        assert!(!read(&p).0.eraser);
    }

    #[test]
    fn the_pointer_number_and_the_time_pass_through() {
        let p = InkPoint {
            pointer_id: 42,
            time_ms: 0xFFFF_FF00,
            ..InkPoint::default()
        };
        let v = read(&p).0;
        assert_eq!(v.pointer_id, 42);
        assert_eq!(v.time_ms, 0xFFFF_FF00);
    }

    #[test]
    fn the_position_maps_himetric_through_the_device_rect_to_the_display_rect() {
        // 画面の幅 1920・高さ 1080 を、機器の 1024 四方へ。比は 1.875 と 1.0546875（どちらも 2 進で割り切れる）
        let p = InkPoint {
            himetric: [512, 512],
            pixel: [7, 7],
            rects: Some((
                RECT_1024,
                InkRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
            )),
            ..InkPoint::default()
        };
        assert_eq!(read(&p).1, [960.0, 540.0]);
        // 比は 0.5 と 0.5。機器の矩形の左上（500, 100）は引かず、himetric の位置にそのまま比を掛けて画面の矩形の左上（100, 50）に足す
        // （100 + 1000 * 0.5 = 600、50 + 400 * 0.5 = 250）
        let p = InkPoint {
            himetric: [1000, 400],
            rects: Some((
                InkRect {
                    left: 500,
                    top: 100,
                    right: 2500,
                    bottom: 1100,
                },
                InkRect {
                    left: 100,
                    top: 50,
                    right: 1100,
                    bottom: 550,
                },
            )),
            ..InkPoint::default()
        };
        assert_eq!(read(&p).1, [600.0, 250.0]);
    }

    #[test]
    fn the_position_multiplies_by_the_ratio_after_dividing_the_sizes() {
        // 比が 2 進で割り切れない（1920 / 1023 と 1080 / 1000）: 比を先に作ってから掛ける順の値。掛けてから割る順なら、最後の桁が違う
        // （x は 20.64516129032258、y は 7.56）。ビットまで比べる
        let p = InkPoint {
            himetric: [11, 7],
            rects: Some((
                InkRect {
                    left: 0,
                    top: 0,
                    right: 1023,
                    bottom: 1000,
                },
                InkRect {
                    left: 0,
                    top: 0,
                    right: 1920,
                    bottom: 1080,
                },
            )),
            ..InkPoint::default()
        };
        let [x, y] = read(&p).1;
        assert_eq!(x.to_bits(), 0x4034_a529_4a52_94a6, "{x}");
        assert_eq!(y.to_bits(), 0x401e_3d70_a3d7_0a3e, "{y}");
    }

    #[test]
    fn a_display_left_of_the_primary_gives_a_negative_position() {
        // 画面の矩形が主のモニターの左上にある（負）: 比は 2 と 2
        let p = InkPoint {
            himetric: [512, 128],
            pixel: [-1, -1],
            rects: Some((
                InkRect {
                    left: 0,
                    top: 0,
                    right: 1024,
                    bottom: 256,
                },
                InkRect {
                    left: -2048,
                    top: -512,
                    right: 0,
                    bottom: 0,
                },
            )),
            ..InkPoint::default()
        };
        assert_eq!(read(&p).1, [-1024.0, -256.0]);
        // 矩形が無く、画素の位置が負でもそのまま
        let p = InkPoint {
            pixel: [-30, -4],
            ..InkPoint::default()
        };
        assert_eq!(read(&p).1, [-30.0, -4.0]);
    }

    #[test]
    fn without_a_usable_device_rect_the_pixel_position_is_used() {
        let display = InkRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let pixel = |rects| {
            read(&InkPoint {
                himetric: [512, 512],
                pixel: [-30, 40],
                rects,
                ..InkPoint::default()
            })
            .1
        };
        // 矩形が読めなかった
        assert_eq!(pixel(None), [-30.0, 40.0]);
        // 幅 0
        let zero_width = InkRect {
            right: 0,
            bottom: 1024,
            ..InkRect::default()
        };
        assert_eq!(pixel(Some((zero_width, display))), [-30.0, 40.0]);
        // 高さ 0
        let zero_height = InkRect {
            right: 1024,
            bottom: 0,
            ..InkRect::default()
        };
        assert_eq!(pixel(Some((zero_height, display))), [-30.0, 40.0]);
        // 右が左より小さい（逆向き）
        let inverted = InkRect {
            left: 1024,
            top: 0,
            right: 0,
            bottom: 1024,
        };
        assert_eq!(pixel(Some((inverted, display))), [-30.0, 40.0]);
    }

    #[test]
    fn the_values_join_a_client_position_into_a_pen_sample() {
        let p = InkPoint {
            pen_mask: PEN_MASK_PRESSURE | PEN_MASK_ROTATION | PEN_MASK_TILT_X | PEN_MASK_TILT_Y,
            pressure: 768,
            tilt_x: 10,
            tilt_y: -20,
            rotation: 400,
            pointer_flags: 0x2 | 0x4,
            pen_flags: 0x1 | 0x4,
            pointer_id: 5,
            time_ms: 1234,
            pixel: [10, 20],
            ..InkPoint::default()
        };
        let (values, screen) = read(&p);
        assert_eq!(screen, [10.0, 20.0]);
        let s = values.with_pos([9.5, 19.25]);
        assert_eq!(
            s,
            PenSample {
                pos: [9.5, 19.25],
                pressure: 0.75,
                tilt: Tilt { x: 10.0, y: -20.0 },
                rotation: Some(40.0),
                contact: true,
                eraser: true,
                barrel: true,
                pointer_id: 5,
                time_ms: 1234,
            }
        );
    }
}
