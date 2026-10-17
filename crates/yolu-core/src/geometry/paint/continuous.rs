//! 3D の面のストロークの連続吹き付け（2D と同じ時計 [`crate::brush::ContinuousClock`]）: この入力で距離のダブを並べなかったなら、
//! 時刻の間隔ごとに、今の筆の点（手ぶれ補正の後）へダブを並べる。線の長さは描いた区間の長さ、向きは最後に測った線の向き。

use super::*;

impl SurfaceStroke {
    /// 入力 1 つの後の連続吹き付け（`dabs_before` はこの入力を受ける前に並べたダブの数）。並べたダブは、ほかのダブと同じく待ち行列で塗る。
    pub(super) fn add_continuous(&mut self, time: f64, dabs_before: u64) {
        let placed = self.dabs != dabs_before;
        let Some(clock) = self.continuous.as_mut() else {
            return;
        };
        let n = clock.due(time, placed);
        if n == 0 {
            return;
        }
        let pen = self.pen;
        let dab = self.locate(ScreenDab {
            at: pen.at,
            pressure: pen.pressure,
            arc: self.sampler.length(),
            direction: self.sampler.direction(),
            tilt: pen.tilt,
            rotation: pen.rotation,
            speed: pen.speed,
        });
        self.dabs += u64::from(n);
        for _ in 0..n {
            self.queue.push_back(dab);
        }
    }

    /// 抜き・入りのために塗れずに待っているダブのバイトを、1 回のストロークの予算（後補正で覚えた入力と合わせて）に数え、超えたら
    /// ストロークを取り消す断りにする。連続吹き付けは、止めて押している間も、塗れないダブを 1 秒に 60 個ずつ待ち行列へ足し続ける。
    pub(super) fn ensure_held_room(&self, doc: &Document) -> Result<(), SurfaceStrokeError> {
        if self.continuous.is_none() {
            return Ok(());
        }
        // 塗れないダブは待ち行列の後ろに続けて並ぶ
        let held = self
            .queue
            .iter()
            .rev()
            .take_while(|d| !self.paintable(d))
            .count();
        let bytes = (held * std::mem::size_of::<QueuedDab>()) as u64 + self.post_bytes();
        if bytes > self.memory_total(doc) {
            return Err(CoreError::StrokeBudgetExceeded.into());
        }
        Ok(())
    }
}
