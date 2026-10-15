//! D3D12 のシェーダーのコンパイラーの選び。
//!
//! wgpu の既定は `Auto`（組み込んだ DXC があればそれ、無ければ PATH の `dxcompiler.dll`、それも無ければ FXC。FXC は古く遅い）。
//! Windows の x86_64 msvc のビルドには DXC を静的に組み込み（`Cargo.toml` の `static-dxc`）、ここで使えると確かめてから使う。
//! 使えなければ FXC に戻る（組み込みの DXC がある `Auto` は、それが作れないと FXC に戻らず D3D12 ごと使えなくなるため）。
//! 環境変数 `WGPU_DX12_COMPILER` があれば、確かめずにその指定に従う。ほかの OS・ビルドでは wgpu の既定（`Auto`）のまま。
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

#[cfg(not(all(windows, target_env = "msvc", target_arch = "x86_64")))]
fn choose() -> wgpu::Dx12Compiler {
    wgpu::Dx12Compiler::from_env().unwrap_or_default()
}

#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn choose() -> wgpu::Dx12Compiler {
    // 環境変数 `WGPU_BACKEND` が D3D12 を外していれば、D3D12 の装置は作られない
    let dx12_excluded =
        wgpu::Backends::from_env().is_some_and(|b| !b.intersects(wgpu::Backends::DX12));
    decide(
        wgpu::Dx12Compiler::from_env(),
        dx12_excluded,
        probe_static_dxc,
    )
}

/// 選びの規則（Windows の x86_64 msvc の `choose` が使う。OS によらずコンパイルされ、単体テストで確かめる）。
/// 環境変数の指定（`env`）があればそれ（確かめない）。D3D12 が外されていれば既定のまま（確かめない）。
/// それ以外は `probe` が通れば静的な DXC、失敗すれば FXC（理由を出す）。
#[cfg_attr(
    not(all(windows, target_env = "msvc", target_arch = "x86_64")),
    allow(dead_code)
)]
fn decide(
    env: Option<wgpu::Dx12Compiler>,
    dx12_excluded: bool,
    probe: impl FnOnce() -> Result<(), String>,
) -> wgpu::Dx12Compiler {
    if let Some(compiler) = env {
        return compiler;
    }
    if dx12_excluded {
        return wgpu::Dx12Compiler::default();
    }
    match probe() {
        Ok(()) => wgpu::Dx12Compiler::StaticDxc,
        Err(reason) => {
            eprintln!("D3D12 のシェーダーのコンパイラー: FXC を使う（{reason}）");
            wgpu::Dx12Compiler::Fxc
        }
    }
}

/// 組み込みの DXC で D3D12 の instance を作れ、アダプターが見えるかを確かめる。組み込みの DXC が作れないと、wgpu は D3D12 の
/// backend ごと作らない（アダプターが 0 個になる）。wgpu は backend の作成の失敗を `log::debug` に落とすので、原因は取れず、
/// D3D12 のアダプターがそもそも無い PC でも同じ 0 個になる（そのときは FXC でも同じ結果）。理由の文はその両方を言う。
#[cfg(all(windows, target_env = "msvc", target_arch = "x86_64"))]
fn probe_static_dxc() -> Result<(), String> {
    let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
    descriptor.backends = wgpu::Backends::DX12;
    descriptor.backend_options.dx12.shader_compiler = wgpu::Dx12Compiler::StaticDxc;
    let instance = wgpu::Instance::new(descriptor);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::DX12));
    if adapters.is_empty() {
        Err(
            "D3D12 のアダプターが見えない。組み込みの DXC の読み込みに失敗した可能性もある"
                .to_string(),
        )
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn probed(result: Result<(), String>) -> (wgpu::Dx12Compiler, bool) {
        let called = Cell::new(false);
        let compiler = decide(None, false, || {
            called.set(true);
            result
        });
        (compiler, called.get())
    }

    #[test]
    fn a_passing_probe_selects_the_static_dxc() {
        let (compiler, called) = probed(Ok(()));
        assert!(called);
        assert!(matches!(compiler, wgpu::Dx12Compiler::StaticDxc));
    }

    #[test]
    fn a_failing_probe_falls_back_to_fxc() {
        let (compiler, called) = probed(Err("理由".to_string()));
        assert!(called);
        assert!(matches!(compiler, wgpu::Dx12Compiler::Fxc));
    }

    #[test]
    fn the_environment_choice_wins_without_probing() {
        for env in [
            wgpu::Dx12Compiler::Fxc,
            wgpu::Dx12Compiler::StaticDxc,
            wgpu::Dx12Compiler::DynamicDxc {
                dxc_path: "dxcompiler.dll".to_string(),
            },
        ] {
            let expected = format!("{env:?}");
            let compiler = decide(Some(env), false, || panic!("環境変数があれば確かめない"));
            assert_eq!(format!("{compiler:?}"), expected);
        }
    }

    #[test]
    fn excluded_d3d12_keeps_the_default_without_probing() {
        let compiler = decide(None, true, || panic!("D3D12 が外れていれば確かめない"));
        assert_eq!(
            format!("{compiler:?}"),
            format!("{:?}", wgpu::Dx12Compiler::default())
        );
    }
}
