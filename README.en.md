[日本語](README.md)

# YoluPainter

[![CI](https://github.com/YozoraKurage/YoluPainter/actions/workflows/main-tested.yml/badge.svg?branch=main)](https://github.com/YozoraKurage/YoluPainter/actions/workflows/main-tested.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

YoluPainter is a painting application for textures on a 2D canvas and on 3D models.
Connect it to Unity to open a model of the scene with its material values, and to apply what you paint to the Unity materials.

Windows is the primary platform. Mac and Linux support is experimental.

## Features

- 2D and 3D painting: brushes that respond to pressure and tilt, color mixing, blur, smudge and clone, symmetry, rulers, and stencils
- Paint, Edit and Pose modes: in the 3D view, select and move projection boxes, decals, points, 3D paths and pose bones
- Keyboard, mouse and pie menu assignments that you can change in the Keyboard Shortcuts category of the settings window
- Layers, groups, masks, clipping, 26 blend modes, adjustment layers, text layers, and locks
- Material painting: Color, Roughness, Metallic, Height, Normal, Emission, and user channels in a single stroke
- Filters, generators that read baked mesh maps (AO (ambient occlusion), curvature, thickness, ID, and more), noise and grunge, and smart materials
- The lilToon look in the 3D view
- Live Link with Unity: open a scene model in one step and send exported textures back to Unity (they are assigned to materials only after you confirm in Unity)
- Editing layers, masks, and effects from the command line (`yolupainter-cli`) and from AI assistants connected over MCP
- PSD layers, groups, masks, and adjustments in and out; import of ABR and CLIP STUDIO (.sut) brushes
- Export of per-channel PNGs and templates for Unity Standard, URP, HDRP, and lilToon
- Automatic recovery after a crash

## Download

The Windows (64-bit) installer and zip are on the [Releases](https://github.com/YozoraKurage/YoluPainter/releases) page.
See [docs/en/INSTALL.md](docs/en/INSTALL.md) for installer options and updates.

An experimental zip for Mac (`yolupainter-<version>-macos-universal-experimental.zip`) is attached too. It holds a universal `YoluPainter.app` with code for both Apple Silicon and Intel.
It has no Apple Developer signature or notarization (only an ad-hoc signature), and the app does not update itself: when a new version is out, an item in the Help menu opens its release page.
On an Apple Silicon Mac (macOS 27), only launching, writing settings and logs, the signature and the universal structure have been checked. The window display and drawing have not been checked, and it has not been run on an Intel Mac.

Because it is unsigned, macOS blocks `YoluPainter.app` the first time you open it after unzipping. To open it, follow these steps ([Apple's guide](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac)):

1. Double-click `YoluPainter.app`. It is blocked; close the message.
2. Choose Apple menu > System Settings, then click Privacy & Security in the sidebar.
3. Go to the Security section, then click Open.
4. Click Open Anyway (it is available for about an hour after you try to open the app).
5. Enter your login password, then click OK. From then on, double-click opens it.

## Building from source

You need stable Rust and a C/C++ build environment.

```sh
cargo build --release -p yolu-app --locked
```

See [docs/en/BUILDING.md](docs/en/BUILDING.md) for each operating system, and [docs/DEVELOPMENT.md](https://github.com/YozoraKurage/YoluPainter/blob/main/docs/DEVELOPMENT.md) (Japanese) for tests.

## Documentation

- [User Guide](docs/en/GUIDE.md)
- [Working with Unity](docs/en/UNITY.md) (Live Link)
- [Command line and AI assistants](docs/en/CLI.md) (`yolupainter-cli`, [connecting over MCP](docs/en/MCP.md))
- In Japanese: [PSD](docs/PSD.md), [brushes](docs/BRUSH.md), [brush import](docs/BRUSH_IMPORT.md), [sub tools](docs/SUBTOOLS.md), [gradient map](docs/GRADIENT_MAP.md), [3D view](docs/PREVIEW.md), [recovery](docs/RECOVERY.md), [save for distribution](docs/SAVE_FOR_DISTRIBUTION.md), [the .ylp format](docs/YLP_FORMAT.md)
- [Changelog](https://github.com/YozoraKurage/YoluPainter/blob/main/CHANGELOG.md)

## Contact

For questions, development discussion, and feature requests, join us on [Discord](https://discord.gg/c8NNfhJ94J).

## Privacy

The application sends nothing over the network except a query to GitHub for the latest version when you choose to check for updates. Live Link exchanges files in a folder on the same PC. External operation listens on `127.0.0.1` (this PC only), and only while it is turned on in the settings.

## License

[MIT License](LICENSE). Licenses for the libraries, fonts, and icons are listed in [THIRD_PARTY.md](THIRD_PARTY.md), and the full texts ship with the distributions.
