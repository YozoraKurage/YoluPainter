//! 連続吹き付け（[`TipShape::continuous`]）: ペンを押している間、筆の点が間隔の距離まで動かなくても、時刻の間隔
//! [`CONTINUOUS_INTERVAL`] ごとに今の筆の点へダブを置く。時計は入力の時刻（秒）で進み、この入力で距離のダブを 1 つでも並べたら、
//! 時計をその入力の時刻へ合わせる（動いている線は今までどおり距離の間隔で置き、止まっている・遅い所だけに時刻のダブが足される。
//! 遅い線ほどダブが多く濃い）。時刻が進まない入力（同じ入力を 2 回渡す）ではダブを足さない。同じ入力の列なら同じダブの列になる。
//! 2D のストローク（[`StrokeState`]）と 3D の面のストローク（`geometry::SurfaceStroke`）が同じ時計を持つ。

use super::*;

/// 時刻のダブの間隔（秒）。画面の更新（60 Hz）と同じ刻みにし、ペンを 1 秒止めると 60 個置く。
pub const CONTINUOUS_INTERVAL: f64 = 1.0 / 60.0;
/// 1 つの入力で置く時刻のダブの上限（入力が 1 秒より長く途切れたときに、まとめて置き過ぎない）。超えた分は置かずに時計を今へ合わせる。
pub const MAX_CONTINUOUS_PER_INPUT: u32 = 60;

/// 連続吹き付けの時計（最後にダブを置いた時刻）。まだ入力を受けていなければ None。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ContinuousClock {
    last: Option<f64>,
}

impl ContinuousClock {
    /// 押した時刻 `time` で、押した点のダブを置いた形（3D の面のストロークは押した点のダブを始めに置く）。
    pub(crate) fn started(time: f64) -> ContinuousClock {
        ContinuousClock { last: Some(time) }
    }

    /// 時刻 `time` の入力を受けた後に置く、時刻のダブの数。`placed` はこの入力で距離のダブを並べたか（並べたら時計を今へ合わせて 0）。
    pub(crate) fn due(&mut self, time: f64, placed: bool) -> u32 {
        let Some(last) = self.last.filter(|_| !placed) else {
            self.last = Some(self.last.map_or(time, |l| l.max(time)));
            return 0;
        };
        if time.is_nan() || time <= last {
            return 0;
        }
        let n = ((time - last) / CONTINUOUS_INTERVAL).floor();
        if n.is_nan() || n < 1.0 {
            return 0;
        }
        if n > MAX_CONTINUOUS_PER_INPUT as f64 {
            self.last = Some(time);
            return MAX_CONTINUOUS_PER_INPUT;
        }
        self.last = Some(last + n * CONTINUOUS_INTERVAL);
        n as u32
    }
}

impl StrokeState {
    /// 入力 1 つの後の連続吹き付け: この入力で距離のダブを並べなかった（`dabs_before` から増えていない）なら、時刻の間隔ごとに今の筆の点
    /// （手ぶれ補正の後の最後の点）へダブを置く。線の長さはここまでに描いた長さ、向きは最後の区間の向き。
    pub(super) fn add_continuous(
        &mut self,
        surface: &mut Surface,
        time: f64,
        dabs_before: u64,
        changed: &mut Vec<TileCoord>,
    ) -> Result<bool, CoreError> {
        let placed = self.dabs != dabs_before;
        let Some(clock) = self.continuous.as_mut() else {
            return Ok(false);
        };
        let n = clock.due(time, placed);
        let Some(pen) = self.last_pen_point.filter(|_| n > 0 && self.has_sample) else {
            return Ok(false);
        };
        let mut any = false;
        for _ in 0..n {
            any |= self.emit(
                surface,
                PendingDab {
                    x: pen.x,
                    y: pen.y,
                    pressure: pen.pressure,
                    arc: self.stroke_length,
                    direction: self.direction,
                    tilt_x: pen.tilt.x,
                    tilt_y: pen.tilt.y,
                    rotation: pen.rotation,
                    speed: pen.speed,
                },
                changed,
            )?;
        }
        Ok(any)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_counts_whole_intervals_and_waits_for_time_to_move() {
        let mut c = ContinuousClock::default();
        assert_eq!(c.due(1.0, true), 0, "最初の入力は時計を合わせるだけ");
        assert_eq!(c.due(1.0, false), 0, "時刻が進まなければ置かない");
        assert_eq!(c.due(1.0 + CONTINUOUS_INTERVAL * 0.5, false), 0);
        assert_eq!(c.due(1.0 + CONTINUOUS_INTERVAL * 2.5, false), 2);
        assert_eq!(c.due(1.0 + CONTINUOUS_INTERVAL * 3.5, false), 1);
        // 距離のダブを並べた入力は、時計を今へ合わせる
        assert_eq!(c.due(2.0, true), 0);
        assert_eq!(c.due(2.0 + CONTINUOUS_INTERVAL * 1.5, false), 1);
        // 長く途切れた入力は上限まで
        assert_eq!(c.due(100.0, false), MAX_CONTINUOUS_PER_INPUT);
        assert_eq!(c.due(100.0, false), 0);
    }
}
