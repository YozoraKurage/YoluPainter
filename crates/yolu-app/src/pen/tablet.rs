//! macOS のタブレットの点を `PenSample` に直す（純粋な部分）。OS の呼び出し（アプリの NSEvent の監視）は `mac_tablet`。ここは macOS 専用にせず、Linux でも単体試験できる。
//!
//! Wacom・XP-Pen などのドライバーは、ペンの動きを普通の NSEvent にして流す: マウスのイベント（左・右・その他のボタンの押し・ドラッグ・離しと移動）に、
//! `subtype` が「タブレットの点」で付き、`pressure`（0〜1）・`tilt`（各軸 −1〜1）・`rotation`（度）・`buttonMask`（ペン先・サイドボタン）が入る。
//! ペンが近づいた・離れたは `tabletProximity` で、ペン先か消しゴムの端かが `pointingDeviceType` で分かる。動かさずに押した直後などは、マウスのイベントが無く
//! `tabletPoint` のイベントだけが来る。メーカーごとの SDK は使わない。
//!
//! 値の直し方:
//! - 位置: `locationInWindow` は点で、原点は左下。ビューの高さで上下を返し、ウィンドウの倍率（`backingScaleFactor`）を掛けて、クライアント領域の物理の画素（左上が原点）にする。
//!   数でない位置の点は、位置を 0 にして詰めずに捨てる（呼ぶ側が、マウスの今の位置から取り直してからここへ渡す）。
//! - 傾き: `tilt` の各軸は −1〜1 に正規化された値。度に直す倍率は 60（傾きの範囲を ±60° と見た値。ドライバーが −1〜1 に正規化する元の角度は確かめていない）。
//!   x は右へ倒すと正で、Windows の `tiltX` と同じ。y は符号を返す: Windows の `tiltY` は「利用者の側（タブレットの下側）へ倒すと正」で、Apple の文書は `tilt.y` が正なら
//!   下（利用者の側）とあるので、文書の向きのままなら Windows と同じになる。ここでは文書とは逆の向き（符号を返した値）にしている。広く使われている別の実装が符号を返していて、
//!   ドライバーが実際に送る向きは文書と食い違うとみたため。**実機では未確認**。
//! - 回転: `rotation` は反時計回りの度とみて、時計回りの 0〜360 にする（未確認）。送らないペンは 0 なので、0 は None（core も回転が 0 のときは「情報なし」と見なす）。
//! - 消しゴムの端: `tabletProximity` で近づいたときの `pointingDeviceType` を `deviceID` ごとに覚え、次の点から付ける。離れたら忘れる。
//! - 触れている: 左ボタンの押し・ドラッグは触れている、離しは触れていない、移動は浮いている。右・その他のボタンと `tabletPoint` は `buttonMask` のペン先
//!   （`tabletPoint` はさらに、左ボタンを押している間。ウィンドウの枠のドラッグなどの入れ子の追跡では、離しのイベントが監視に届かないので、移動で押していない印に戻し、
//!   OS の今の左ボタンの状態も合わせて見る）。サイドボタンは `buttonMask` の下・上のサイドボタン、または右・その他のボタンの押し（ドライバーがサイドボタンに
//!   割り当てたクリック）。

use std::collections::HashMap;

use super::PenSample;
use crate::engine::Tilt;

/// NSEvent の `buttonMask` のビット（`NSEventButtonMask`）。
pub const MASK_TIP: u64 = 1;
pub const MASK_LOWER_SIDE: u64 = 2;
pub const MASK_UPPER_SIDE: u64 = 4;

/// 正規化された傾き（1.0）が指す角度（度）。
pub const TILT_DEGREES: f32 = 60.0;

/// `NSEventType`・`NSEventSubtype` の値（macOS の試験が objc2-app-kit の定数と同じであることを確かめる）。
pub mod ns {
    pub const LEFT_MOUSE_DOWN: u64 = 1;
    pub const LEFT_MOUSE_UP: u64 = 2;
    pub const RIGHT_MOUSE_DOWN: u64 = 3;
    pub const RIGHT_MOUSE_UP: u64 = 4;
    pub const MOUSE_MOVED: u64 = 5;
    pub const LEFT_MOUSE_DRAGGED: u64 = 6;
    pub const RIGHT_MOUSE_DRAGGED: u64 = 7;
    pub const TABLET_POINT: u64 = 23;
    pub const TABLET_PROXIMITY: u64 = 24;
    pub const OTHER_MOUSE_DOWN: u64 = 25;
    pub const OTHER_MOUSE_UP: u64 = 26;
    pub const OTHER_MOUSE_DRAGGED: u64 = 27;
    pub const SUBTYPE_TABLET_POINT: i16 = 1;
    pub const SUBTYPE_TABLET_PROXIMITY: i16 = 2;
}

/// 点の出どころ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointKind {
    /// 左ボタンの押し・ドラッグ（ペン先で触れている）。
    LeftDown,
    LeftUp,
    /// 右・その他のボタンの押し・ドラッグ・離し（ドライバーがサイドボタンに割り当てたクリックなど）。
    OtherDown,
    OtherUp,
    /// ボタンを押さない移動（浮いている）。
    Hover,
    /// `tabletPoint` のイベント（マウスのイベントを伴わない点）。
    Tablet,
}

/// 受けたイベントの扱い。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Input {
    Point(PointKind),
    Proximity,
}

/// イベントの種類から扱いを決める。タブレットのイベントでなければ None（マウスのイベントは、`subtype` がタブレットの点・近づいた離れたのときだけ）。
/// `subtype` はマウスのイベントのときだけ読む（ほかの種類のイベントには無い）ので、関数で渡す。
pub fn classify(event_type: u64, subtype: impl FnOnce() -> i16) -> Option<Input> {
    let kind = match event_type {
        ns::TABLET_POINT => return Some(Input::Point(PointKind::Tablet)),
        ns::TABLET_PROXIMITY => return Some(Input::Proximity),
        ns::LEFT_MOUSE_DOWN | ns::LEFT_MOUSE_DRAGGED => PointKind::LeftDown,
        ns::LEFT_MOUSE_UP => PointKind::LeftUp,
        ns::RIGHT_MOUSE_DOWN
        | ns::RIGHT_MOUSE_DRAGGED
        | ns::OTHER_MOUSE_DOWN
        | ns::OTHER_MOUSE_DRAGGED => PointKind::OtherDown,
        ns::RIGHT_MOUSE_UP | ns::OTHER_MOUSE_UP => PointKind::OtherUp,
        ns::MOUSE_MOVED => PointKind::Hover,
        _ => return None,
    };
    match subtype() {
        ns::SUBTYPE_TABLET_POINT => Some(Input::Point(kind)),
        ns::SUBTYPE_TABLET_PROXIMITY => Some(Input::Proximity),
        _ => None,
    }
}

/// NSEvent から読んだ、タブレットの点 1 つ（値は OS のまま。直しは `TabletState::point`）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub kind: PointKind,
    /// ビュー（ウィンドウの内容）の中の位置（点）。原点は左下。
    pub location: [f64; 2],
    /// ビューの高さ（点）。
    pub height: f64,
    /// ウィンドウの倍率（`backingScaleFactor`）。
    pub scale: f64,
    pub pressure: f32,
    /// −1〜1。
    pub tilt: [f32; 2],
    /// 反時計回りの度。
    pub rotation: f32,
    /// `buttonMask`。
    pub buttons: u64,
    /// OS の今の左ボタンの状態（`NSEvent.pressedMouseButtons` の最下位のビット）。`tabletPoint` の「左ボタンを押している間」を確かめる。
    pub left_pressed: bool,
    pub device: u64,
    /// 起動してからの秒。
    pub timestamp: f64,
}

/// クライアント領域の物理の画素（左上が原点）の位置。数でない値（位置・高さ・倍率）があれば None。
pub fn client_position(location: [f64; 2], height: f64, scale: f64) -> Option<[f32; 2]> {
    let finite = [location[0], location[1], height, scale]
        .iter()
        .all(|v| v.is_finite());
    (finite && scale > 0.0).then(|| {
        [
            (location[0] * scale) as f32,
            ((height - location[1]) * scale) as f32,
        ]
    })
}

/// 正規化された傾き（各軸 −1〜1）を度にする。x は右へ倒すと正。y は Apple の文書とは逆の向き（符号を返す）で、Windows の `tiltY`（利用者の側へ倒すと正）に
/// 合わせたつもりの向き。実機では未確認。
pub fn tilt_degrees(tilt: [f32; 2]) -> Tilt {
    let axis = |v: f32| {
        if v.is_finite() {
            v.clamp(-1.0, 1.0) * TILT_DEGREES
        } else {
            0.0
        }
    };
    Tilt {
        x: axis(tilt[0]),
        // Apple の文書（y が正なら下＝利用者の側）とは逆の向きにしている（実機では未確認）
        y: -axis(tilt[1]),
    }
}

/// 反時計回りの度を、時計回りの 0〜360 にする。0 は送らないペン（None）。
pub fn clockwise_rotation(rotation: f32) -> Option<f32> {
    if !rotation.is_finite() || rotation == 0.0 {
        return None;
    }
    Some((360.0 - rotation).rem_euclid(360.0))
}

/// 起動してからの秒を、ミリ秒（u32。Windows の `dwTime` と同じく 2^32 で一周する）にする。
pub fn milliseconds(timestamp: f64) -> u32 {
    if !timestamp.is_finite() || timestamp < 0.0 {
        return 0;
    }
    ((timestamp * 1000.0) as u64 & 0xFFFF_FFFF) as u32
}

#[derive(Default)]
struct Device {
    /// 近づいたときの種類が消しゴムの端か。
    eraser: bool,
    /// 左ボタンを押している（`tabletPoint` の「触れている」に使う）。
    left_down: bool,
    /// 触れている最後の点と、それを詰めた先の目印（離れたとき、離しの点を作る）。
    touching: Option<(usize, PenSample)>,
}

/// ペンごとの状態（近づいたときの種類・触れているか）。`deviceID` で分ける。
#[derive(Default)]
pub struct TabletState {
    devices: HashMap<u64, Device>,
}

fn release(owner: usize, sample: PenSample) -> (usize, PenSample) {
    (
        owner,
        PenSample {
            contact: false,
            pressure: 0.0,
            barrel: false,
            ..sample
        },
    )
}

impl TabletState {
    /// ペンが近づいた・離れた。種類（消しゴムの端か）は近づいたときに覚え、離れたら忘れる。触れたままの点があれば、その離しの点（と、詰めた先の目印）を返す
    /// （離しのイベントが届かなかったとき、ペンの押しが残らないように）。
    pub fn proximity(
        &mut self,
        device: u64,
        entering: bool,
        eraser: bool,
    ) -> Option<(usize, PenSample)> {
        let previous = self.devices.remove(&device);
        if entering {
            self.devices.insert(
                device,
                Device {
                    eraser,
                    ..Device::default()
                },
            );
        }
        previous
            .and_then(|d| d.touching)
            .map(|(owner, sample)| release(owner, sample))
    }

    /// 点を `PenSample` にする。`owner` は詰める先の目印（離れたときの離しの点を同じ先へ返す）。位置が数でない点は詰めない（None）。ただし触れていた点への
    /// 触れていない点なら、最後の位置の離しの点を返す（離しの点が来なくて、ペンの押しが残らないように）。
    pub fn point(&mut self, owner: usize, point: &Point) -> Option<PenSample> {
        let device = self.devices.entry(point.device).or_default();
        match point.kind {
            PointKind::LeftDown => device.left_down = true,
            // 移動は、左ボタンを押していない印（入れ子の追跡で離しが届かなかったときの残りを消す）。`tabletPoint` は OS の今の状態と合わせる
            PointKind::LeftUp | PointKind::Hover => device.left_down = false,
            PointKind::Tablet => device.left_down &= point.left_pressed,
            PointKind::OtherDown | PointKind::OtherUp => {}
        }
        let tip = point.buttons & MASK_TIP != 0;
        let contact = match point.kind {
            PointKind::LeftDown => true,
            PointKind::LeftUp | PointKind::Hover => false,
            PointKind::OtherDown | PointKind::OtherUp => tip,
            PointKind::Tablet => tip || device.left_down,
        };
        let barrel = point.buttons & (MASK_LOWER_SIDE | MASK_UPPER_SIDE) != 0
            || point.kind == PointKind::OtherDown;
        let pressure = if point.pressure.is_finite() {
            point.pressure.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let Some(pos) = client_position(point.location, point.height, point.scale) else {
            return if contact {
                None
            } else {
                device
                    .touching
                    .take()
                    .map(|(owner, last)| release(owner, last).1)
            };
        };
        let sample = PenSample {
            pos,
            pressure,
            tilt: tilt_degrees(point.tilt),
            rotation: clockwise_rotation(point.rotation),
            contact,
            eraser: device.eraser,
            barrel,
            pointer_id: point.device as u32,
            time_ms: milliseconds(point.timestamp),
        };
        device.touching = contact.then_some((owner, sample));
        Some(sample)
    }

    /// 触れている全部のペンの離しの点（機能を切ったとき、ペンの押しが残らないように）。状態は触れていないことにする。
    pub fn release_all(&mut self) -> Vec<(usize, PenSample)> {
        self.devices
            .values_mut()
            .filter_map(|d| {
                d.left_down = false;
                d.touching.take()
            })
            .map(|(owner, sample)| release(owner, sample))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 位置が数の点（試験の点は、どれも数）を詰める。
    fn at(state: &mut TabletState, owner: usize, p: &Point) -> PenSample {
        state.point(owner, p).expect("位置は数")
    }

    fn point(kind: PointKind) -> Point {
        Point {
            kind,
            location: [10.0, 20.0],
            height: 100.0,
            scale: 1.0,
            pressure: 0.5,
            tilt: [0.0, 0.0],
            rotation: 0.0,
            buttons: 0,
            left_pressed: true,
            device: 0,
            timestamp: 1.5,
        }
    }

    #[test]
    fn the_position_is_flipped_by_the_view_height_and_scaled_to_physical_pixels() {
        // 左下が原点の点（10, 20）を、高さ 100 のビューの左上が原点の位置へ（80）
        assert_eq!(
            client_position([10.0, 20.0], 100.0, 1.0),
            Some([10.0, 80.0])
        );
        // Retina（倍率 2）は、点ではなく物理の画素
        assert_eq!(
            client_position([10.0, 20.0], 100.0, 2.0),
            Some([20.0, 160.0])
        );
        // 小数の位置も保つ
        assert_eq!(
            client_position([10.25, 99.75], 100.0, 2.0),
            Some([20.5, 0.5])
        );
        // 上の端と下の端
        assert_eq!(client_position([0.0, 100.0], 100.0, 2.0), Some([0.0, 0.0]));
        assert_eq!(client_position([0.0, 0.0], 100.0, 2.0), Some([0.0, 200.0]));
        // 数でない値は None（0 にして左下へ飛ばさない）
        assert_eq!(client_position([f64::NAN, 5.0], 100.0, 1.0), None);
        assert_eq!(client_position([5.0, f64::INFINITY], 100.0, 1.0), None);
        assert_eq!(client_position([5.0, 5.0], f64::NAN, 1.0), None);
        assert_eq!(client_position([5.0, 5.0], 100.0, f64::NAN), None);
        assert_eq!(client_position([5.0, 5.0], 100.0, 0.0), None, "倍率 0");
    }

    #[test]
    fn a_point_without_a_position_is_dropped_but_a_lift_is_not_lost() {
        let mut state = TabletState::default();
        let mut lost = point(PointKind::LeftDown);
        lost.location = [f64::NAN, f64::NAN];
        // 位置の無い点は詰めない（左下へ飛ばない）
        assert_eq!(state.point(0, &lost), None);
        // 触れている最中に位置の無い点が来ても、最後の位置のまま。離しの点が位置を失ったら、最後の位置の離しを返す
        let touched = at(&mut state, 3, &point(PointKind::LeftDown));
        assert!(touched.contact);
        let mut hover_lost = point(PointKind::Hover);
        hover_lost.location = [f64::NAN, 0.0];
        let lift = state.point(3, &hover_lost).expect("離しの点");
        assert_eq!(lift.pos, touched.pos, "最後の位置");
        assert!(!lift.contact);
        // 離しはもう返さない（押しは残っていない）
        assert_eq!(state.point(3, &hover_lost), None);
        assert_eq!(state.proximity(0, false, false), None);
    }

    #[test]
    fn the_tilt_is_scaled_to_degrees_and_y_follows_the_windows_direction() {
        assert_eq!(tilt_degrees([0.0, 0.0]), Tilt { x: 0.0, y: 0.0 });
        // x は右へ倒すと正（Windows の tiltX と同じ）
        assert_eq!(tilt_degrees([1.0, 0.0]).x, 60.0);
        assert_eq!(tilt_degrees([-0.5, 0.0]).x, -30.0);
        // y は NSEvent と符号が逆（利用者の側へ倒すと Windows の tiltY は正）
        assert_eq!(tilt_degrees([0.0, -1.0]).y, 60.0);
        assert_eq!(tilt_degrees([0.0, 0.5]).y, -30.0);
        // 範囲の外・数でない値は、Windows の範囲（−90〜90）に収まる
        let wild = tilt_degrees([5.0, -5.0]);
        assert!(wild.x.abs() <= 90.0 && wild.y.abs() <= 90.0, "{wild:?}");
        assert_eq!(tilt_degrees([f32::NAN, f32::INFINITY]).x, 0.0);
        assert_eq!(tilt_degrees([f32::NAN, f32::INFINITY]).y, 0.0);
    }

    #[test]
    fn the_rotation_is_none_when_not_sent_and_clockwise_otherwise() {
        assert_eq!(clockwise_rotation(0.0), None);
        assert_eq!(clockwise_rotation(f32::NAN), None);
        // 反時計回りの 90° は、時計回りの 270°
        assert_eq!(clockwise_rotation(90.0), Some(270.0));
        assert_eq!(clockwise_rotation(-90.0), Some(90.0));
        assert_eq!(clockwise_rotation(360.0), Some(0.0));
        let turned = clockwise_rotation(400.0).unwrap();
        assert!((0.0..360.0).contains(&turned), "{turned}");
    }

    #[test]
    fn the_timestamp_becomes_milliseconds_that_wrap_like_windows() {
        assert_eq!(milliseconds(1.5), 1500);
        assert_eq!(milliseconds(-1.0), 0);
        assert_eq!(milliseconds(f64::NAN), 0);
        // 2^32 ミリ秒（4294967.296 秒）を越えたら一周する
        assert_eq!(milliseconds(4_294_968.0), 704);
    }

    #[test]
    fn only_tablet_events_are_taken_and_a_mouse_is_left_alone() {
        use ns::*;
        let mouse = || 0i16; // 普通のマウス（subtype 0）
        for t in [
            LEFT_MOUSE_DOWN,
            LEFT_MOUSE_UP,
            LEFT_MOUSE_DRAGGED,
            RIGHT_MOUSE_DOWN,
            RIGHT_MOUSE_UP,
            RIGHT_MOUSE_DRAGGED,
            OTHER_MOUSE_DOWN,
            OTHER_MOUSE_UP,
            OTHER_MOUSE_DRAGGED,
            MOUSE_MOVED,
        ] {
            assert_eq!(classify(t, mouse), None, "type {t}");
            // トラックパッドなどの別の subtype（Touch = 3）も詰めない
            assert_eq!(classify(t, || 3), None, "type {t}");
            assert!(
                matches!(classify(t, || SUBTYPE_TABLET_POINT), Some(Input::Point(_))),
                "type {t}"
            );
            assert_eq!(
                classify(t, || SUBTYPE_TABLET_PROXIMITY),
                Some(Input::Proximity)
            );
        }
        // tabletPoint・tabletProximity の型のイベントは、subtype を読まずに取る
        let never = || panic!("subtype を読まない");
        assert_eq!(
            classify(TABLET_POINT, never),
            Some(Input::Point(PointKind::Tablet))
        );
        assert_eq!(classify(TABLET_PROXIMITY, never), Some(Input::Proximity));
        // キー・スクロールなど、監視に入らない種類
        assert_eq!(classify(10, || SUBTYPE_TABLET_POINT), None);
        assert_eq!(classify(22, || SUBTYPE_TABLET_POINT), None);
        // 種類ごとの行き先
        assert_eq!(
            classify(LEFT_MOUSE_DRAGGED, || SUBTYPE_TABLET_POINT),
            Some(Input::Point(PointKind::LeftDown))
        );
        assert_eq!(
            classify(MOUSE_MOVED, || SUBTYPE_TABLET_POINT),
            Some(Input::Point(PointKind::Hover))
        );
        assert_eq!(
            classify(OTHER_MOUSE_UP, || SUBTYPE_TABLET_POINT),
            Some(Input::Point(PointKind::OtherUp))
        );
    }

    #[test]
    fn a_pressure_of_zero_is_kept_and_other_pressures_stay_in_range() {
        let mut state = TabletState::default();
        let mut down = point(PointKind::LeftDown);
        down.pressure = 0.0;
        let sample = at(&mut state, 0, &down);
        assert!(sample.contact, "触れた直後の筆圧 0 の点も触れている点");
        assert_eq!(sample.pressure, 0.0);
        down.pressure = 1.7;
        assert_eq!(at(&mut state, 0, &down).pressure, 1.0);
        down.pressure = f32::NAN;
        assert_eq!(at(&mut state, 0, &down).pressure, 0.0);
        down.pressure = 0.25;
        assert_eq!(at(&mut state, 0, &down).pressure, 0.25);
    }

    #[test]
    fn touching_follows_the_left_button_and_the_pen_tip() {
        let mut state = TabletState::default();
        assert!(!at(&mut state, 0, &point(PointKind::Hover)).contact);
        assert!(at(&mut state, 0, &point(PointKind::LeftDown)).contact);
        // tabletPoint は、左ボタンを押している間は触れている（buttonMask が無くても）
        assert!(at(&mut state, 0, &point(PointKind::Tablet)).contact);
        assert!(!at(&mut state, 0, &point(PointKind::LeftUp)).contact);
        assert!(!at(&mut state, 0, &point(PointKind::Tablet)).contact);
        // マウスのイベントを伴わない点は、buttonMask のペン先
        let mut tip = point(PointKind::Tablet);
        tip.buttons = MASK_TIP;
        assert!(at(&mut state, 0, &tip).contact);
        // 右・その他のボタンは、ペン先の触れで決める（サイドボタンだけなら浮いている）
        let mut side = point(PointKind::OtherDown);
        assert!(!at(&mut state, 0, &side).contact);
        side.buttons = MASK_TIP | MASK_LOWER_SIDE;
        assert!(at(&mut state, 0, &side).contact);
    }

    #[test]
    fn a_hover_after_a_lost_lift_ends_the_touch_for_the_next_tablet_point() {
        // ウィンドウの枠のドラッグなど入れ子の追跡では、離しが監視に届かず、左ボタンを押した記憶が残る。浮かせた点（移動）で消える
        let mut state = TabletState::default();
        assert!(at(&mut state, 0, &point(PointKind::LeftDown)).contact);
        assert!(!at(&mut state, 0, &point(PointKind::Hover)).contact);
        assert!(
            !at(&mut state, 0, &point(PointKind::Tablet)).contact,
            "浮いているペンの tabletPoint は触れていない"
        );
        // 移動が来ないまま tabletPoint だけ来ても、OS の左ボタンが離れていれば触れていない
        assert!(at(&mut state, 0, &point(PointKind::LeftDown)).contact);
        let mut released = point(PointKind::Tablet);
        released.left_pressed = false;
        assert!(!at(&mut state, 0, &released).contact);
        // その後に左ボタンが押されたと言っても、押した記憶は戻らない（次の押しのイベントで戻る）
        assert!(!at(&mut state, 0, &point(PointKind::Tablet)).contact);
        assert!(at(&mut state, 0, &point(PointKind::LeftDown)).contact);
        assert!(at(&mut state, 0, &point(PointKind::Tablet)).contact);
        // ペン先の触れは、左ボタンの状態によらない
        let mut tip = point(PointKind::Tablet);
        tip.left_pressed = false;
        tip.buttons = MASK_TIP;
        assert!(at(&mut state, 0, &tip).contact);
    }

    #[test]
    fn the_side_buttons_are_the_barrel() {
        let mut state = TabletState::default();
        assert!(!at(&mut state, 0, &point(PointKind::LeftDown)).barrel);
        for mask in [MASK_LOWER_SIDE, MASK_UPPER_SIDE, MASK_TIP | MASK_UPPER_SIDE] {
            let mut p = point(PointKind::Tablet);
            p.buttons = mask;
            assert!(at(&mut state, 0, &p).barrel, "mask {mask}");
        }
        let mut tip_only = point(PointKind::Tablet);
        tip_only.buttons = MASK_TIP;
        assert!(!at(&mut state, 0, &tip_only).barrel);
        // ドライバーがサイドボタンに右ボタンを割り当てたクリックも、サイドボタン
        assert!(at(&mut state, 0, &point(PointKind::OtherDown)).barrel);
        assert!(!at(&mut state, 0, &point(PointKind::OtherUp)).barrel);
    }

    #[test]
    fn the_eraser_end_is_remembered_from_the_proximity_for_the_next_points() {
        let mut state = TabletState::default();
        // 近づく前（または知らないペン）はペン先
        assert!(!at(&mut state, 0, &point(PointKind::Hover)).eraser);
        // 消しゴムの端が近づく
        assert_eq!(state.proximity(3, true, true), None);
        let mut hover = point(PointKind::Hover);
        hover.device = 3;
        let floating = at(&mut state, 0, &hover);
        assert!(floating.eraser && !floating.contact);
        assert_eq!(floating.pointer_id, 3);
        let mut down = hover;
        down.kind = PointKind::LeftDown;
        let touched = at(&mut state, 0, &down);
        assert!(touched.eraser && touched.contact);
        // 別のペンには付けない
        assert!(!at(&mut state, 0, &point(PointKind::Hover)).eraser);
        // 離れたら忘れる（触れたままなら、離しの点が返る）
        assert_eq!(state.proximity(3, false, false), Some(release(0, touched)));
        assert!(!at(&mut state, 0, &hover).eraser);
        // ペン先で近づき直したらペン先
        assert_eq!(state.proximity(3, true, false), None);
        assert!(!at(&mut state, 0, &hover).eraser);
        // 消しゴムの端に持ち替えた（離れずに近づく）。前の触れた点の離しは返る
        let touched = at(&mut state, 7, &down);
        assert!(touched.contact && !touched.eraser);
        let (owner, ended) = state.proximity(3, true, true).expect("離しの点");
        assert_eq!(owner, 7);
        assert!(!ended.contact);
        assert!(at(&mut state, 0, &hover).eraser);
    }

    #[test]
    fn leaving_while_touching_gives_one_lift_to_the_window_that_got_the_touch() {
        let mut state = TabletState::default();
        state.proximity(0, true, false);
        let mut p = point(PointKind::LeftDown);
        p.pressure = 0.9;
        p.buttons = MASK_LOWER_SIDE;
        let touched = at(&mut state, 5, &p);
        assert!(touched.contact && touched.barrel);
        let (owner, lift) = state.proximity(0, false, false).expect("離しの点");
        assert_eq!(owner, 5);
        assert_eq!(
            lift,
            PenSample {
                contact: false,
                pressure: 0.0,
                barrel: false,
                ..touched
            }
        );
        // 離しのあとに離れても、何も返らない
        state.proximity(0, true, false);
        at(&mut state, 5, &p);
        at(&mut state, 5, &point(PointKind::LeftUp));
        assert_eq!(state.proximity(0, false, false), None);
        // 触れずに離れても何も返らない
        state.proximity(1, true, false);
        let mut hover = point(PointKind::Hover);
        hover.device = 1;
        at(&mut state, 0, &hover);
        assert_eq!(state.proximity(1, false, false), None);
    }

    #[test]
    fn switching_the_feature_off_releases_every_touching_pen_once() {
        let mut state = TabletState::default();
        let mut a = point(PointKind::LeftDown);
        a.device = 1;
        let mut b = point(PointKind::LeftDown);
        b.device = 2;
        let mut c = point(PointKind::Hover);
        c.device = 3;
        at(&mut state, 10, &a);
        at(&mut state, 20, &b);
        at(&mut state, 30, &c);
        let mut released = state.release_all();
        released.sort_by_key(|(owner, _)| *owner);
        assert_eq!(
            released
                .iter()
                .map(|(o, s)| (*o, s.pointer_id))
                .collect::<Vec<_>>(),
            [(10, 1), (20, 2)]
        );
        assert!(released.iter().all(|(_, s)| !s.contact));
        assert!(state.release_all().is_empty(), "2 度は返さない");
        // 左ボタンを押したままの記憶も消えている
        assert!(!at(&mut state, 0, &point(PointKind::Tablet)).contact);
    }

    #[test]
    fn a_full_sample_carries_position_tilt_rotation_and_time() {
        let mut state = TabletState::default();
        let sample = at(
            &mut state,
            0,
            &Point {
                kind: PointKind::LeftDown,
                location: [30.5, 40.0],
                height: 200.0,
                scale: 2.0,
                pressure: 0.75,
                tilt: [0.5, -0.25],
                rotation: 90.0,
                buttons: MASK_TIP,
                left_pressed: true,
                device: 4,
                timestamp: 12.5,
            },
        );
        assert_eq!(
            sample,
            PenSample {
                pos: [61.0, 320.0],
                pressure: 0.75,
                tilt: Tilt { x: 30.0, y: 15.0 },
                rotation: Some(270.0),
                contact: true,
                eraser: false,
                barrel: false,
                pointer_id: 4,
                time_ms: 12500,
            }
        );
    }
}
