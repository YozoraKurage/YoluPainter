//! GPU 合成の試験で使うデバイス。製品と同じ上限で動かせるアダプターだけを選ぶ。
use std::io::Write;
use std::sync::{Mutex, MutexGuard, OnceLock};

use eframe::egui_wgpu::{wgpu, WgpuSetup, WgpuSetupExisting};

static DEVICE: OnceLock<Result<WgpuSetupExisting, String>> = OnceLock::new();
static TEST: Mutex<()> = Mutex::new(());

/// 窓を捨てるまで保持する。共有デバイスのエラースコープとキューを試験どうしで混ぜない。
/// 窓の貸し出し（`gpu_thread::lease`。harness を持つほかの試験と同じもの）を先に取る: この装置は kittest の共用の接続とは別の
/// Instance・Device なので、貸し出しを通さないと、harness を持つ試験と同時に別々の装置を作ってしまう（lavapipe の中で落ちる向き）。
/// 取る順は貸し出し → `TEST` で、どの試験もこの順。
pub fn begin(name: &str) -> Option<MutexGuard<'static, ()>> {
    super::gpu_thread::lease();
    let guard = TEST.lock().unwrap_or_else(|e| e.into_inner());
    match DEVICE.get_or_init(select_device) {
        Ok(_) => Some(guard),
        Err(reason) => {
            // libtest の成功時の出力捕捉を通さず、省略を通常の cargo test でも残す。
            let _ = writeln!(std::io::stderr(),
                "省略: canvas_gpu::{name}: {reason}。未検証: この試験の GPU 合成・表示と CPU との比較、切替・復帰の保証。");
            None
        }
    }
}

pub fn renderer() -> egui_kittest::wgpu::WgpuTestRenderer {
    let setup = DEVICE
        .get()
        .expect("begin を先に呼ぶ")
        .as_ref()
        .expect("利用可能な GPU")
        .clone();
    let state = egui_kittest::wgpu::create_render_state(
        WgpuSetup::Existing(setup),
        super::render_options(),
    );
    egui_kittest::wgpu::WgpuTestRenderer::from_render_state(state)
}

fn select_device() -> Result<WgpuSetupExisting, String> {
    pollster::block_on(async {
        // 環境変数があればその範囲を守る。既定は Vulkan / Metal / DX12 を GL より先に調べる。
        let backends =
            wgpu::Backends::from_env().unwrap_or(wgpu::Backends::PRIMARY | wgpu::Backends::GL);
        let mut reasons = Vec::new();
        for backend in [
            wgpu::Backends::VULKAN,
            wgpu::Backends::METAL,
            wgpu::Backends::DX12,
            wgpu::Backends::GL,
        ] {
            if !backends.intersects(backend) {
                continue;
            }
            let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
            descriptor.backends = backend;
            let instance = wgpu::Instance::new(descriptor);
            let mut adapters = instance.enumerate_adapters(backend).await;
            adapters.sort_by_key(|a| a.get_info().device_type != wgpu::DeviceType::Cpu);
            if adapters.is_empty() {
                reasons.push(format!("{backend:?}: アダプターなし"));
            }
            for adapter in adapters {
                let info = adapter.get_info();
                let WgpuSetup::CreateNew(defaults) = egui_kittest::wgpu::default_wgpu_setup()
                else {
                    unreachable!()
                };
                let descriptor = (defaults.device_descriptor)(&adapter);
                let limits = &descriptor.required_limits;
                // GL の egui デバイスは WebGL2 相当。アダプター単体の能力だけでは判定できない。
                if !adapter
                    .get_downlevel_capabilities()
                    .flags
                    .contains(wgpu::DownlevelFlags::COMPUTE_SHADERS)
                    || limits.max_storage_buffers_per_shader_stage < 5
                    || limits.max_storage_textures_per_shader_stage < 1
                    || limits.max_compute_invocations_per_workgroup < 64
                {
                    reasons.push(format!(
                        "{:?}/{}: egui デバイスの compute/storage 上限が GPU 合成に足りない",
                        info.backend, info.name
                    ));
                    continue;
                }
                match adapter.request_device(&descriptor).await {
                    Ok((device, queue)) => {
                        let _ = writeln!(
                            std::io::stderr(),
                            "canvas_gpu: {:?}/{} ({:?})",
                            info.backend,
                            info.name,
                            info.device_type
                        );
                        return Ok(WgpuSetupExisting {
                            instance,
                            adapter,
                            device,
                            queue,
                        });
                    }
                    Err(error) => {
                        reasons.push(format!("{:?}/{}: {error}", info.backend, info.name))
                    }
                }
            }
        }
        Err(format!("使える描画器なし（{}）", reasons.join("; ")))
    })
}
