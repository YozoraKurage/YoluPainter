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

Pen tablets are read through Windows Ink by default, so enable Windows Ink in the tablet driver too (to read them through WinTab instead, choose it with Pen input under Pen in Edit → Settings…). The pen buttons in the brush's Tool Properties select which settings respond to pressure. If the pen's pressure is too strong or too weak, adjust it in Pen Pressure under Pen in Edit → Settings….

## Mac (experimental)

Install Xcode Command Line Tools and Rust, and run on a system with Metal support.

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

To build the distribution form (a zip holding a universal `YoluPainter.app` with code for both Apple Silicon and Intel), prepare Xcode Command Line Tools (`lipo`, `codesign`, `iconutil`, `sips`, `ditto` and `plutil`), Python 3.10 or later, and the two Rust targets, then run these at the repository root.

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
cargo xtask build --target universal-apple-darwin --release
cargo xtask bundle --target universal-apple-darwin
```

This makes `target/dist/yolupainter-<version>-macos-universal-experimental.zip`. It builds for the two targets and joins them with `lipo` (the minimum macOS is set to 11.0 for both), makes the icon from the existing logo, signs the bundle ad hoc (`codesign -s -`), and zips it with `ditto`. There is no Apple Developer signature or notarization. How to open an unsigned `.app` is in [“Mac (experimental)” in Download and updates](INSTALL.md#mac-experimental).

For pen tablets (Wacom, XP-Pen and others), the app reads the pressure, tilt, eraser end and side buttons that the driver sends as standard macOS events (experimental). No manufacturer SDK is used. If something is off, turn off Tablet pressure (experimental) under Pen in Edit → Settings…, and the pen draws like a mouse. If the pen's pressure is too strong or too weak, adjust it in Pen Pressure under Pen in Edit → Settings….

## Linux (experimental)

Prepare a C/C++ compiler, `pkg-config`, an X11 desktop, and a GPU driver. The window opens through X11, so on a Wayland desktop it runs on XWayland (it cannot start on a Wayland-only environment without XWayland). File dialogs require a D-Bus session, `xdg-desktop-portal`, and a portal backend for your desktop. Yes/no confirmations (such as discarding unsaved changes) need `zenity`; without it, actions that need the confirmation are cancelled. The interface font (BIZ UDPGothic) is embedded in the executable, so a system Japanese font is not required.

Example package names on Debian/Ubuntu are `build-essential`, `pkg-config`, `libxkbcommon-dev`, `libwayland-dev` (the Wayland parts are still part of the build, so it is needed to build), `libvulkan1`, `xdg-desktop-portal`, `xdg-desktop-portal-gtk`, and `zenity`. Use the appropriate GPU driver for your hardware.

```sh
cargo build --release -p yolu-app --locked
./target/release/yolupainter
```

You can paint with a mouse. Pressure input depends on the desktop environment and input device.
