//! D3D12 のシェーダーのコンパイラーの選び。
//!
//! wgpu の既定は FXC（古く遅い）。Windows の x86_64 msvc のビルドには DXC を静的に組み込み（`Cargo.toml` の `static-dxc`）、
//! ここで使えると確かめてから使う。使えなければ FXC に戻る（wgpu の `Auto` は組み込んだ DXC が作れないと D3D12 ごと使えなくなるため）。
//! 環境変数 `WGPU_DX12_COMPILER` があれば、確かめずにその指定に従う。ほかの OS・ビルドでは wgpu の既定のまま。
use std::sync::OnceLock;

/// 製品と試験が装置を作るときに使う、D3D12 のシェーダーのコンパイラー（プロセスで 1 度だけ決める）。
pub fn dx12_compiler() -> wgpu::Dx12Compiler {
    static CHOICE: OnceLock<wgpu::Dx12Compiler> = OnceLock::new();
    CHOICE.get_or_init(choose).clone()
}

/// `wgpu::Instance` を作る設定（環境変数の指定を読み、D3D12 のコンパイラーを `dx12_compiler()` にする）。
pub fn instance_descriptor() -> wgpu::InstanceDescriptor {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    descriptor.backend_options.dx12.shader_compiler = dx12_compiler();
    descriptor
}

fn choose() -> wgpu::Dx12Compiler {
    wgpu::Dx12Compiler::from_env().unwrap_or_else(choose_without_env)
}

#[cfg(not(all(windows, target_env = "msvc", target_arch = "x86_64")))]
fn choose_without_env() -> wgpu::Dx12Compiler {
    wgpu::Dx12Compiler::default()
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn choose_without_env() -> wgpu::Dx12Compiler {
    // 環境変数 `WGPU_BACKEND` が D3D12 を外していれば、確かめずに既定のまま（D3D12 の装置は作られない）
    if wgpu::Backends::from_env().is_some_and(|b| !b.intersects(wgpu::Backends::DX12)) {
        return wgpu::Dx12Compiler::default();
    }
    match probe_static_dxc() {
        Ok(()) => wgpu::Dx12Compiler::StaticDxc,
        Err(reason) => {
            eprintln!("D3D12 のシェーダーのコンパイラー: 組み込みの DXC を使えないため FXC を使う（{reason}）");
            wgpu::Dx12Compiler::Fxc
        }
    }
}

/// 組み込みの DXC で D3D12 の instance を作れ、アダプターが見えるかを確かめる。組み込みの DXC が作れないと、wgpu は D3D12 の
/// backend ごと作らない（アダプターが 0 個になる）。D3D12 のアダプターがそもそも無い PC でも 0 個になり、そのときは FXC でも同じ。
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn probe_static_dxc() -> Result<(), String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::StaticDxc;
    let instance = wgpu::Instance::new(descriptor);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    if adapters.is_empty() {
        Err("D3D12 のアダプターが見つからない".to_string())
    } else {
        Ok(())
    }
}
