//! 連続吹き付けのブラシで描いている間の、押したまま止めているペン・マウスの点（2D のキャンバスと 3D ビューで同じ）。ペンを止めている間は
//! egui の入力の点が来ないので、新しい点が来なかったフレームに、最後の点の位置・筆圧・傾き・回転で今の時刻の点を 1 つ渡す。
//!
//! 点の時刻は入力の時計（ペンは OS の時刻、マウスは egui の時刻）なので、フレームの時刻（egui）とは時計が違う。最後の点を受けた
//! フレームの時刻を覚え、そこからのフレームの時刻の進みを、最後の点の時刻に足す（入力の時計のまま進む）。

use egui::Pos2;

use crate::engine::Tilt;

/// 最後に渡した点（画面の点）と、その時刻（入力の時計）と、受けたときのフレームの時刻（egui の秒）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeldPoint {
    pub pos: Pos2,
    pub pressure: f32,
    pub tilt: Tilt,
    /// 度（回転を送れないペン・マウスは None）。
    pub rotation: Option<f32>,
    pub time: f64,
    /// 受けたときのフレームの時刻（まだ決めていなければ None）。
    pub received: Option<f64>,
}

impl HeldPoint {
    pub fn new(pos: Pos2, pressure: f32, tilt: Tilt, rotation: Option<f32>, time: f64) -> Self {
        HeldPoint {
            pos,
            pressure,
            tilt,
            rotation,
            time,
            received: None,
        }
    }

    /// フレームの時刻 `now` に渡す点の時刻（入力の時計）。受けたフレームの時刻をまだ決めていなければ None（このフレームに受けた点）。
    pub fn time_at(&self, now: f64) -> Option<f64> {
        let received = self.received?;
        Some(self.time + (now - received).max(0.0))
    }
}
