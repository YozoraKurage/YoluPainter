//! GPU・CPU の切り替え。GPU が使えない・壊れている・予算を超える・時間切れのときは理由を返して CPU の `bake` に戻る。
use super::{BakeAdapter, BakeGpu, GpuBakeError, GpuBakeOptions, GpuBakeStats};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};
use yolu_core::mesh_maps::{
    bake, MeshBakeBudget, MeshBakeInput, MeshBakeResult, MeshBakeSettings, MeshMapError,
};

/// ベイクを行う場所の選び方。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BakeBackend {
    /// 使えるハードウェアの GPU があれば GPU、なければ CPU（ソフトウェアの GPU は使わない）。
    #[default]
    Auto,
    /// GPU（ソフトウェアの描画も許す）。使えなければ理由つきで CPU。
    Gpu,
    Cpu,
}

/// GPU を選べる設定なのに CPU で焼いた理由の種類（画面は種類から言語ごとの短い文を作り、`BakeRun::fallback_reason` の詳細はツールチップに出す）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FallbackKind {
    /// アダプター・デバイス・シェーダーが使えない（自動のときはハードウェアの GPU が無い場合を含む）。
    Unavailable,
    /// 入力・出力が予算かデバイスの上限を超える。
    Budget,
    /// 実行中の失敗（デバイスの消失・時間切れ・検証エラー・量子化の前の NaN・無限大・範囲外の値）。
    Failed,
}

/// 実際に使った場所と、GPU を選んだのに CPU で焼いた理由。
#[derive(Clone, Debug)]
pub struct BakeRun {
    pub requested: BakeBackend,
    /// GPU で焼いたとき、そのアダプターと計測。準備の途中で止まって GPU が一度も動かなかった取消・時間切れは None
    /// （`fallback_kind` も None）で、どこでも焼いていない。
    pub gpu: Option<(BakeAdapter, GpuBakeStats)>,
    /// GPU を選べる設定なのに CPU で焼いたときの理由の種類（`Cpu` を選んだときは None）。
    pub fallback_kind: Option<FallbackKind>,
    /// その詳細（日本語の文。`fallback_kind` と同時に入る）。
    pub fallback_reason: Option<String>,
}
impl BakeRun {
    pub fn used_gpu(&self) -> bool {
        self.gpu.is_some()
    }
}

enum Slot {
    Empty,
    Ready(Box<BakeGpu>),
    /// 作れなかった理由。`allow_software` を許していない状態で作れなかったので、許せば作れるかもしれない。
    Unavailable {
        allow_software: bool,
        /// 作ったときの ray query の入切（切り替わったら作り直す。ray query つきの準備で作れなかった GPU が、切れば作れることがある）。
        ray_query: bool,
        reason: String,
    },
}

/// `BakeGpu`（デバイスとシェーダー）を使い回す入れ物。別のスレッドから共有できる。
/// ベイクの間は中身を貸し出すので、その間の `probe` と `bake` は終わりを待つ。
pub struct GpuBakeSlot {
    options: GpuBakeOptions,
    /// ray query を使うか（`GpuBakeOptions::ray_query` の今の値。`set_ray_query` で変える）。
    ray_query: AtomicBool,
    /// 焼きの途中で ray query の道が失敗したので止めている理由。入切が変わるまで compute だけで焼く。
    blocked: Mutex<Option<String>>,
    slot: Mutex<Slot>,
}
impl Default for GpuBakeSlot {
    fn default() -> Self {
        Self::new(GpuBakeOptions::default())
    }
}
impl GpuBakeSlot {
    pub fn new(options: GpuBakeOptions) -> Self {
        Self {
            ray_query: AtomicBool::new(options.ray_query),
            blocked: Mutex::new(None),
            options,
            slot: Mutex::new(Slot::Empty),
        }
    }
    /// ray query を使うかを切り替える（アプリの設定）。デバイスの作り方が変わるので、値が変わったら次の `probe`・`bake` で作り直す。
    /// 値が変わると、途中の失敗で止めていた ray query も試し直す。
    pub fn set_ray_query(&self, on: bool) {
        if self.ray_query.swap(on, Ordering::Relaxed) != on {
            *self.blocked.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }
    /// 設定している ray query の入切（途中の失敗で止めているかは `ray_query_block`）。
    pub fn ray_query(&self) -> bool {
        self.ray_query.load(Ordering::Relaxed)
    }
    /// ray query を止めている理由（焼きの途中で ray query の道が失敗したとき）。
    pub fn ray_query_block(&self) -> Option<String> {
        self.blocked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn block_ray_query(&self, reason: String) {
        *self.blocked.lock().unwrap_or_else(|e| e.into_inner()) = Some(reason);
    }
    /// 今、デバイスを ray query つきで作る入切（設定が入で、止めていないとき入）。
    fn effective_ray_query(&self) -> bool {
        self.ray_query() && self.ray_query_block().is_none()
    }
    /// アダプターが使えるか（まだ作っていなければ作る）。`allow_software` は `BakeBackend::Gpu` のとき true。
    pub fn probe(&self, allow_software: bool) -> Result<BakeAdapter, String> {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure(&mut slot, allow_software)
            .map(|g| g.adapter().clone())
    }
    /// 試験用: 今のデバイスを壊れた状態にする（作っていなければ何もしない）。
    #[doc(hidden)]
    pub fn fail_for_test(&self, reason: &str) {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Slot::Ready(gpu) = &mut *slot {
            gpu.fail_for_test(reason);
        }
    }
    /// 試験用: 次の `bake` を、レイをたどる道を決めたあとに失敗させる（作っていなければ何もしない）。
    #[doc(hidden)]
    pub fn fail_midway_for_test(&self, reason: &str, kind: super::MidFailure) {
        let mut slot = self.slot.lock().unwrap_or_else(|e| e.into_inner());
        if let Slot::Ready(gpu) = &mut *slot {
            gpu.fail_midway_for_test(reason, kind);
        }
    }
    fn ensure<'a>(
        &self,
        slot: &'a mut Slot,
        allow_software: bool,
    ) -> Result<&'a mut BakeGpu, String> {
        let ray_query = self.effective_ray_query();
        let rebuild = match slot {
            Slot::Empty => true,
            Slot::Unavailable {
                allow_software: tried,
                ray_query: tried_ray_query,
                ..
            } => (allow_software && !*tried) || *tried_ray_query != ray_query,
            // 壊れたものは作り直す。ソフトウェアのアダプターを許さない呼び出しには使わない。
            Slot::Ready(gpu) => {
                gpu.failure().is_some()
                    || (gpu.adapter().software && !allow_software)
                    || gpu.ray_query_option() != ray_query
            }
        };
        if rebuild {
            *slot = match BakeGpu::new(GpuBakeOptions {
                allow_software,
                ray_query,
                ..self.options.clone()
            }) {
                Ok(mut gpu) => {
                    if let Some(reason) = self.ray_query_block() {
                        gpu.block_ray_query(&reason);
                    }
                    Slot::Ready(Box::new(gpu))
                }
                Err(e) => Slot::Unavailable {
                    allow_software,
                    ray_query,
                    reason: e.to_string(),
                },
            };
        }
        match slot {
            Slot::Ready(gpu) => Ok(gpu),
            Slot::Unavailable { reason, .. } => Err(reason.clone()),
            Slot::Empty => unreachable!("直前に作った"),
        }
    }
}

/// GPU が準備のあとで使えなくなって CPU に戻るときの、CPU に渡す予算。GPU で使った時間（準備を含む）を `max_seconds` から引く
/// （引き切ったら、CPU が最初の確認で時間切れにするだけの小さい正の値）。0（制限なし）はそのまま。
fn remaining_budget(budget: &MeshBakeBudget, elapsed: Duration) -> MeshBakeBudget {
    MeshBakeBudget {
        max_seconds: if budget.max_seconds > 0. {
            (budget.max_seconds - elapsed.as_secs_f64()).max(1e-9)
        } else {
            0.
        },
        ..budget.clone()
    }
}

/// 選んだ場所でメッシュマップを焼く。進捗・取消・時間切れは CPU の `bake` と同じ（GPU でも CPU でも空の結果で知らせる）。
/// CPU も断る入力（UV・設定・予算・手動の ID 色）は `Err` で、CPU へ戻さずそのまま返す。
///
/// GPU が準備（BVH・曲率などの CPU の重い処理）のあとで予算超過・失敗になると、CPU の `bake` を最初から呼ぶので、準備をもう一度行い、
/// 進捗は "Preparing" に戻る。時間制限は GPU で使った時間（準備を含む）を引いた残りを CPU に渡すので、合計で制限を超えない。
#[allow(clippy::too_many_arguments)]
pub fn bake_mesh_maps(
    backend: BakeBackend,
    gpu: &GpuBakeSlot,
    input: &MeshBakeInput,
    settings: &MeshBakeSettings,
    budget: &MeshBakeBudget,
    cancel: Option<&AtomicBool>,
    reference: Option<&MeshBakeInput>,
    mut progress: impl FnMut(f64, &str) -> bool,
) -> Result<(MeshBakeResult, BakeRun), MeshMapError> {
    let started = Instant::now();
    let mut fallback = None;
    if backend != BakeBackend::Cpu {
        let allow_software = backend == BakeBackend::Gpu;
        let mut slot = gpu.slot.lock().unwrap_or_else(|e| e.into_inner());
        // ray query の道で焼いている途中の失敗（ドライバーの固まり・待ち時間切れ・デバイスの消失ではないもの）は、ray query を止めて
        // compute だけで 1 回やり直す。やり直しても失敗したら、CPU に戻る。
        let mut retried = false;
        loop {
            match gpu.ensure(&mut slot, allow_software) {
                Ok(g) => match g.bake(input, settings, budget, cancel, reference, &mut progress) {
                    Ok(baked) => {
                        let adapter = g.adapter().clone();
                        // dispatch が 1 回も無い = 準備の途中で止まった。GPU で焼いたとは言わない
                        let ran = baked.stats.dispatches > 0;
                        return Ok((
                            baked.result,
                            BakeRun {
                                requested: backend,
                                gpu: ran.then_some((adapter, baked.stats)),
                                fallback_kind: None,
                                fallback_reason: None,
                            },
                        ));
                    }
                    Err(GpuBakeError::Refused(e)) => return Err(e),
                    Err(e) => {
                        if !retried
                            && matches!(e, GpuBakeError::Failed(_))
                            && g.failed_on_ray_query_only()
                        {
                            retried = true;
                            gpu.block_ray_query(e.to_string());
                            continue;
                        }
                        let kind = match &e {
                            GpuBakeError::Budget(_) => FallbackKind::Budget,
                            GpuBakeError::Failed(_) => FallbackKind::Failed,
                            GpuBakeError::Unavailable(_) | GpuBakeError::Refused(_) => {
                                FallbackKind::Unavailable
                            }
                        };
                        fallback = Some((kind, e.to_string()));
                    }
                },
                Err(reason) => fallback = Some((FallbackKind::Unavailable, reason)),
            }
            break;
        }
    }
    let remaining;
    let budget = if fallback.is_some() {
        remaining = remaining_budget(budget, started.elapsed());
        &remaining
    } else {
        budget
    };
    let result = bake(input, settings, budget, cancel, reference, progress)?;
    Ok((
        result,
        BakeRun {
            requested: backend,
            gpu: None,
            fallback_kind: fallback.as_ref().map(|f| f.0),
            fallback_reason: fallback.map(|f| f.1),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_ray_query_remakes_a_slot_that_could_not_be_made_and_clears_a_block() {
        let slot = GpuBakeSlot::new(GpuBakeOptions {
            ray_query: true,
            ..Default::default()
        });
        // ray query つきで作れなかった印の入ったスロット（ray query を切れば作れる GPU がある）
        let mut state = Slot::Unavailable {
            allow_software: false,
            ray_query: true,
            reason: "前の理由".into(),
        };
        let _ = slot.ensure(&mut state, false);
        assert!(
            matches!(&state, Slot::Unavailable { reason, .. } if reason == "前の理由"),
            "入が同じなのに作り直した"
        );
        let mut state = Slot::Unavailable {
            allow_software: false,
            ray_query: true,
            reason: "前の理由".into(),
        };
        slot.set_ray_query(false);
        let _ = slot.ensure(&mut state, false);
        assert!(
            !matches!(&state, Slot::Unavailable { reason, .. } if reason == "前の理由"),
            "切に変えたのに作り直さなかった"
        );
        // 途中の失敗で止めた ray query は、設定が変わると試し直す（同じ値の設定し直しでは止めたまま）
        slot.set_ray_query(true);
        slot.block_ray_query("途中で失敗".into());
        assert!(slot.ray_query() && !slot.effective_ray_query());
        slot.set_ray_query(true);
        assert!(slot.ray_query_block().is_some());
        slot.set_ray_query(false);
        slot.set_ray_query(true);
        assert!(slot.ray_query_block().is_none() && slot.effective_ray_query());
    }

    #[test]
    fn the_cpu_gets_what_is_left_of_the_time_limit() {
        let budget = |max_seconds| MeshBakeBudget {
            max_seconds,
            max_bytes: 123,
            max_degree_of_parallelism: 3,
        };
        let secs = Duration::from_secs_f64;
        // 制限なしはそのまま
        assert_eq!(remaining_budget(&budget(0.), secs(5.)).max_seconds, 0.);
        // GPU で使った分を引く（ほかの項目は変えない）
        let left = remaining_budget(&budget(10.), secs(3.5));
        assert!((left.max_seconds - 6.5).abs() < 1e-9);
        assert_eq!((left.max_bytes, left.max_degree_of_parallelism), (123, 3));
        // 使い切ったら 0（= 制限なし）にならず、CPU が最初の確認で時間切れにする小さい正の値
        for used in [10., 11., 1e6] {
            let left = remaining_budget(&budget(10.), secs(used)).max_seconds;
            assert!(left > 0. && left < 1e-6, "{used}: {left}");
        }
    }
}
