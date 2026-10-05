[日本語](README.md)

# YoluPainter

[![CI](https://github.com/YozoraKurage/YoluPainter/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/YozoraKurage/YoluPainter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

YoluPainter is a painting application for textures on a 2D canvas and on 3D models.
Connect it to Unity to paint while seeing your colors on the real materials of an avatar in the scene.

Windows is the primary platform. Mac and Linux support is experimental.

## Features

- 2D and 3D painting: brushes that respond to pressure and tilt, color mixing, blur, smudge and clone, symmetry, and stencils
- Layers, groups, masks, clipping, 26 blend modes, adjustment layers, and locks
- Material painting: Color, Roughness, Metallic, Height, Normal, Emission, and user channels in a single stroke
- Filters, generators that read baked mesh maps (AO, curvature, thickness, ID, and more), noise and grunge, and smart materials
- The lilToon look in the 3D view
- Live Link with Unity: open a model in one step and see your colors on the scene's materials (original assets are never modified)
- PSD layers, groups, masks, and adjustments in and out; import of ABR and CLIP STUDIO (.sut) brushes
- Export of per-channel PNGs and templates for Unity Standard, URP, HDRP, and lilToon
- Automatic recovery after a crash

## Download

The Windows (64-bit) installer and zip are on the [Releases](https://github.com/YozoraKurage/YoluPainter/releases) page.
See [docs/en/INSTALL.md](docs/en/INSTALL.md) for installer options and updates.

## Building from source

You need stable Rust and a C/C++ build environment.

```sh
cargo build --release -p yolu-app --locked
```

See [docs/en/BUILDING.md](docs/en/BUILDING.md) for each operating system, and [docs/DEVELOPMENT.md](https://github.com/YozoraKurage/YoluPainter/blob/main/docs/DEVELOPMENT.md) (Japanese) for tests and the Unity bridge.

## Documentation

- [Features and controls](docs/en/GUIDE.md)
- [Working with Unity](docs/en/UNITY.md) (Live Link, exchanging `.ylp` files with the Unity version)
- In Japanese: [PSD](docs/PSD.md), [brushes](docs/BRUSH.md), [brush import](docs/BRUSH_IMPORT.md), [sub tools](docs/SUBTOOLS.md), [gradient map](docs/GRADIENT_MAP.md), [3D view](docs/PREVIEW.md), [recovery](docs/RECOVERY.md), [window](docs/WINDOW.md), [save for distribution](docs/SAVE_FOR_DISTRIBUTION.md)
- [Changelog](https://github.com/YozoraKurage/YoluPainter/blob/main/CHANGELOG.md)

## Privacy

The application sends nothing over the network except a query to GitHub for the latest version when you choose to check for updates. Live Link only communicates within the same PC.

## License

[MIT License](LICENSE). Licenses for the libraries, fonts, and icons are listed in [THIRD_PARTY.md](THIRD_PARTY.md), and the full texts ship with the distributions.
