//! 描画試験の窓（wgpu の Instance・Device）の作成・使用・破棄を、同時に 1 つの試験だけに絞る。
//!
//! Vulkan（lavapipe）は、別々のスレッドで窓を同時に作ると中（create_bind_group）で落ちることがあった。窓を持つ試験どうしを
//! 重ねないために、3 つの口がある。どれも同じ貸し出し（`WINDOWS`）を取るので、混ぜて使っても重ならない。
//!
//! - [`builder`]（kittest の harness を作る入口）: 試験のスレッドが、最初の harness を作る前に貸し出しを取る。持ったまま試験が終わる
//!   （スレッドが終わる）と放す。`.wgpu()` を呼ばない harness も取る: kittest の既定の描画器は、最初の `render()`（画像の比べ・
//!   `image_snapshot`）で wgpu の装置を遅れて作るので、描くかどうかは builder の形では見分けられない。したがって、画面を描かない
//!   harness（CPU だけの部品の試験）も窓を持つ試験と同時には走らない。貸し出しを取らずに並列のままなのは、kittest の harness を
//!   作らず、GPU の装置も作らない試験（画面を作らない `headless_` の試験など）だけ。
//! - [`run`]（試験の本体を常駐の 1 本のスレッドへ送る）: 窓の作成・使用・破棄を同じスレッドで行う。送っている間だけ貸し出しを持つ。
//! - [`lease`]（直接）: harness を作らなくても GPU の装置を作る試験は、先頭で呼ぶ。製品のスレッドで GPU の確認・ベイクをする試験
//!   （`BakeBackend::Gpu` を選んで焼くなど。装置は製品のスレッドが作る）と、`canvas_device::begin`（中で呼ぶ）。製品のスレッドが試験の中で
//!   終わるなら、試験のスレッドが持てば足りる。`window_lease` の `every_gpu_bake_test_takes_the_lease` が、このような試験の呼び忘れを見つける。
use std::any::Any;
use std::cell::RefCell;
use std::sync::{mpsc, Mutex, MutexGuard, Once, OnceLock, PoisonError};

/// 窓を持っている試験の貸し出し（同時に 1 つ）。
static WINDOWS: Mutex<()> = Mutex::new(());

thread_local! {
    /// このスレッドが持っている貸し出し（試験のスレッドは終わるときに放す。常駐のスレッドはジョブごとに放す）。
    static LEASE: RefCell<Option<MutexGuard<'static, ()>>> = const { RefCell::new(None) };
}

/// 窓を作る前に呼ぶ。ほかのスレッドが窓を持っていれば、その試験が終わるまで待つ。同じスレッドで何度呼んでも 1 回分。
/// 放すのは、スレッドが終わるとき（libtest は試験ごとにスレッドを分ける）か、`run` に送るとき。
pub fn lease() {
    LEASE.with(|slot| {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            *slot = Some(WINDOWS.lock().unwrap_or_else(PoisonError::into_inner));
        }
    });
}

/// このスレッドの貸し出しを放す（持っていなければ何もしない）。
fn release() {
    let _ = LEASE.try_with(|slot| slot.borrow_mut().take());
}

/// 誰も窓を持っていないか（試験が貸し出しの放し忘れを確かめる）。自分が持っていれば false。
pub fn is_free() -> bool {
    WINDOWS.try_lock().is_ok()
}

/// kittest の harness の builder。harness を作る試験は `Harness::builder()` の代わりにこれを使う（ほかの口で作ると、
/// `window_lease` の試験が落ちる）。`.wgpu()` を呼ぶかどうかによらず取る（上の説明）。貸し出しは最初の harness の前に取るので、
/// `.wgpu()` で装置を作る前に重ならない。
pub fn builder<State>() -> egui_kittest::HarnessBuilder<State> {
    lease();
    egui_kittest::Harness::<State>::builder()
}

thread_local! {
    /// このスレッドで最後に起きた panic の場所。ワーカーの出力捕捉先は最初の呼び手に属するので、
    /// libtest の panic フック行（場所つき）は後続の試験には届かない。そのためフックで場所だけ控える。
    static LAST_PANIC_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 既存のフックの前に、panic の場所を控えるだけの層を 1 度だけ重ねる。既存のフックの出力は変えない。
fn record_panic_locations() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if let Some(at) = info.location() {
                let at = format!("{}:{}:{}", at.file(), at.line(), at.column());
                let _ = LAST_PANIC_AT.try_with(|slot| {
                    if let Ok(mut slot) = slot.try_borrow_mut() {
                        *slot = Some(at);
                    }
                });
            }
            previous(info);
        }));
    });
}

/// ワーカーで落ちた試験の panic。呼び手の試験へ戻すときに場所も運ぶ。
pub struct Failure {
    pub payload: Box<dyn Any + Send>,
    pub location: Option<String>,
}

impl Failure {
    /// 失敗の本文と、落ちた場所（ファイル:行:列）。
    pub fn describe(&self) -> String {
        let message = self
            .payload
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| self.payload.downcast_ref::<&str>().copied())
            .unwrap_or("文字列以外の panic");
        match &self.location {
            Some(at) => format!("{message}（{at}）"),
            None => message.to_string(),
        }
    }
}

/// ワーカーで試験を実行し、落ちたら panic の中身と場所を返す。
pub fn run_checked(test: fn()) -> Result<(), Failure> {
    type Job = Box<dyn FnOnce() + Send>;
    static WORKER: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        record_panic_locations();
        let (sender, receiver) = mpsc::channel::<Job>();
        std::thread::Builder::new()
            .name("描画試験".into())
            .spawn(move || {
                for job in receiver {
                    job();
                }
            })
            .expect("描画試験スレッドを作る");
        sender
    });
    let (sender, receiver) = mpsc::sync_channel(1);
    // 呼び手が窓を持っていれば先に放す（常駐のスレッドが貸し出しを待って、呼び手が結果を待つ行き止まりを避ける）
    release();
    worker
        .send(Box::new(move || {
            LAST_PANIC_AT.with(|slot| slot.borrow_mut().take());
            lease();
            // 失敗を呼び出した libtest の試験へ戻し、後続の試験も実行できるようにする。
            let result = std::panic::catch_unwind(test).map_err(|payload| Failure {
                payload,
                location: LAST_PANIC_AT.with(|slot| slot.borrow_mut().take()),
            });
            // 常駐のスレッドは終わらないので、ジョブごとに放す（ほかの試験の窓を止めない）。この試験が控えた一時の物も、ここで消す
            release();
            crate::common::tmp::sweep();
            let _ = sender.send(result);
        }))
        .expect("描画試験スレッドが生きている");
    receiver.recv().expect("描画試験の結果を受け取る")
}

pub fn run(test: fn()) {
    if let Err(failure) = run_checked(test) {
        // ワーカーの出力捕捉先は最初の呼び手に属するので、失敗の本文と場所は今の試験にも残す。
        eprintln!("描画試験に失敗: {}", failure.describe());
        std::panic::resume_unwind(failure.payload);
    }
}
