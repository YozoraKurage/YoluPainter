# Download and updates

[日本語](../INSTALL.md)

Download from [Releases](https://github.com/YozoraKurage/YoluPainter-rs/releases). Windows (64-bit) has two distribution formats.

- `yolupainter-<version>-x86_64-pc-windows-msvc-setup.exe` (installer): installs per user without administrator privileges. The default destination is `%LOCALAPPDATA%\Programs\YoluPainter` (changeable), with a Start menu entry and optional `.ylp` file association. Uninstall through Settings → Apps; you will be asked whether to remove settings, brushes, and recovery data as well (they remain if no prompt is shown). Use `/S` for silent installation, `/ASSOC=1` to enable file association (`/ASSOC=0` to disable it), and `/RUN` to launch the application after installation.
- `yolupainter-<version>-x86_64-pc-windows-msvc.zip`: extract and run `yolupainter.exe`; no installation is needed.

Both include `README.md` and a `docs` folder (English under `docs/en`) with this document, so they can be read offline.

## Updates

On first launch, the application asks whether to check for updates at startup. Only choosing Yes enables a request to GitHub for the latest version on each launch. Change this at any time through Help → Check for Updates at Startup, or check manually through Help → Check for Updates….

When a new version is available, the Help heading shows an indicator. On Windows installed through the installer, “Update to YoluPainter x.y.z” downloads, verifies, and installs the update, asking you to save first if there are unsaved changes. Downloads are used only after verifying the signed update metadata's signature, size, and SHA-256. Portable Windows ZIP distributions and Linux open the release page instead. Applications built from source do not have update menu items; only distribution builds embed the update public key.

## Startup warning (SmartScreen)

Current distributions are not code-signed. If Windows SmartScreen displays “Windows protected your PC,” choose “More info” → “Run anyway” to launch.

## Privacy

The application does not send information over the network except to query GitHub for the latest version when you choose to check for updates (by enabling Check for Updates at Startup or selecting Check for Updates…). The query is an ordinary HTTPS request to fetch an update metadata file. Apart from the application name and version in the User-Agent, no information identifying the user is included. Live Link communicates only within the same machine.
