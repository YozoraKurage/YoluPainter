# Building from source

[日本語](../BUILDING.md)

You need the stable Rust toolchain and a C/C++ build environment. See the [Rust installation instructions](https://doc.rust-lang.org/book/ch01-01-installation.html). Obtain and extract the source, then run the following commands in the folder containing `Cargo.toml`. The first build needs an internet connection to download dependencies.

The display requires a GPU and compatible driver. Rendering uses [wgpu](https://docs.rs/wgpu/30.0.1/wgpu/struct.Backends.html): Direct3D 12 or Vulkan on Windows, Metal on Mac, and Vulkan or other backends on Linux. Unity is not required to run the application on its own.

To use the command line and MCP server (`yolupainter-cli`; [usage](CLI.md)) as well, build it the same way with `-p yolu-cli` in place of `-p yolu-app` (it does not use the graphics libraries, so it builds quickly).

## Windows

Prepare 64-bit Windows, Visual Studio Build Tools with “Desktop development with C++” (including the Windows SDK), and Rust's MSVC toolchain. Run in PowerShell:

```powershell
cargo build --release -p yolu-app --locked --target x86_64-pc-windows-msvc
.\target\x86_64-pc-windows-msvc\release\yolupainter.exe
```

For pen tablets, enable Windows Ink in the tablet driver too. The pen buttons in the brush's Tool Properties select which settings respond to pressure. If the pen's pressure is too strong or too weak, adjust it in View → Pen Pressure….

## Mac (experimental)

Install Xcode Command Line Tools and Rust, and run on a system with Metal support.

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

There is no dedicated pen input handling equivalent to Windows Ink. Mouse operation is the baseline; pressure input depends on the OS and input device.

## Linux (experimental)

Prepare a C/C++ compiler, `pkg-config`, an X11 desktop, and a GPU driver. The window opens through X11, so on a Wayland desktop it runs on XWayland (it cannot start on a Wayland-only environment without XWayland). File dialogs require a D-Bus session, `xdg-desktop-portal`, and a portal backend for your desktop. Yes/no confirmations (such as discarding unsaved changes) need `zenity`; without it, actions that need the confirmation are cancelled. The interface font (BIZ UDPGothic) is embedded in the executable, so a system Japanese font is not required.

Example package names on Debian/Ubuntu are `build-essential`, `pkg-config`, `libxkbcommon-dev`, `libwayland-dev` (the Wayland parts are still part of the build, so it is needed to build), `libvulkan1`, `xdg-desktop-portal`, `xdg-desktop-portal-gtk`, and `zenity`. Use the appropriate GPU driver for your hardware.

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

You can paint with a mouse. Pressure input depends on the desktop environment and input device.
