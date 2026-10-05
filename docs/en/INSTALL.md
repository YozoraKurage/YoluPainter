# Download and updates

[日本語](../INSTALL.md)

Download from [Releases](https://github.com/YozoraKurage/YoluPainter/releases). Windows (64-bit) has two distribution formats.

- `yolupainter-<version>-x86_64-pc-windows-msvc-setup.exe` (installer): installs per user without administrator privileges. The default destination is `%LOCALAPPDATA%\Programs\YoluPainter` (changeable), with a Start menu entry and optional `.ylp` file association. Uninstall through Settings → Apps; you will be asked whether to remove settings and recovery data as well (they remain if no prompt is shown). Things you made are removed in neither case ([table below](#uninstalling-and-your-data)). Use `/S` for silent installation, `/ASSOC=1` to enable file association (`/ASSOC=0` to disable it), and `/RUN` to launch the application after installation. For a silent uninstall use `/S`, and add `/DELETEDATA` to remove settings and recovery data as well.
- `yolupainter-<version>-x86_64-pc-windows-msvc.zip`: extract and run `yolupainter.exe`; no installation is needed.

Both include `README.md` and a `docs` folder (English under `docs/en`) with this document, so they can be read offline.

## Updates

On first launch, the application asks whether to check for updates at startup. Only choosing Yes enables a request to GitHub for the latest version on each launch. Change this at any time through Help → Check for Updates at Startup, or check manually through Help → Check for Updates….

Turning on Help → Use Beta Versions also offers beta versions (versions with an `alpha`, `beta` or `rc` identifier, such as `0.4.0-rc.1`) and recommends the newer of the stable release and the beta. It is off by default, and while off only stable releases are considered. While you run a beta, the status bar shows “Beta” to the left of the version. The app never updates to an older version, so after turning the setting off, nothing is offered until the next stable release is newer than the version you run.

When a new version is available, the Help heading shows an indicator. On Windows installed through the installer, “Update to YoluPainter x.y.z” downloads, verifies, and installs the update, asking you to save first if there are unsaved changes. Downloads are used only after verifying the signed update metadata's signature, size, and SHA-256. Portable Windows ZIP distributions and Linux open the release page instead. Applications built from source do not have update menu items; only distribution builds embed the update public key.

When another window of the installed YoluPainter is open, the update does not start and the app says “Another YoluPainter is running” (the installer cannot replace an executable that is in use). The downloaded installer is kept: close the other windows and press “Update and restart” again to install it right away. In a silent run the installer waits up to 60 seconds for the executable to be released; if it is still in use, it exits without changing anything. The application's own update runs the installer with `/RUN`, and then the application that is already installed is started again (a silent run without `/RUN` does not start it).

## Uninstalling and your data

Uninstalling removes the installed files (the application, documents, shortcut and file association) and the installers downloaded for updates (`%LOCALAPPDATA%\YoluPainter\updates`). Other data is removed only when you answer Yes to “Also delete settings and recovery data?” (or pass `/DELETEDATA` to a silent uninstall), and then only what the table below marks as removed. Things you made are never removed, whichever answer you give.

| Location | Contents | On “delete” |
| --- | --- | --- |
| `%APPDATA%\YoluPainter\settings.conf` | Settings | Removed |
| `%APPDATA%\YoluPainter\recovery.conf`, `update.conf`, `layout.json` | Recovery settings, the update-check and beta choices, panel and window layout | Removed |
| `%APPDATA%\YoluPainter\recovery\` | Recovery generations | Removed |
| `%APPDATA%\YoluPainter\logs\` | Crash records | Removed |
| `%LOCALAPPDATA%\YoluPainter\thumbnails\` | Thumbnail cache | Removed |
| `%LOCALAPPDATA%\YoluPainter\LiveLink\` | Live Link connection files | Removed |
| `%APPDATA%\YoluPainter\Library\` (the default library folder) | Personal library | Kept |
| `%APPDATA%\YoluPainter\brushes\` | Your brushes and erasers | Kept |
| `%APPDATA%\YoluPainter\subtools\` | Your sub-tools | Kept |
| `%APPDATA%\YoluPainter\gradients\` | Gradient sets | Kept |
| `%APPDATA%\YoluPainter\colorsets\` | Color sets | Kept |
| `%APPDATA%\YoluPainter\hide_presets\` | Presets for hiding parts of a model | Kept |
| `%APPDATA%\YoluPainter\pose_presets\` | Pose presets | Kept |

When kept items, or files this application did not create, are present, the `%APPDATA%\YoluPainter` folder stays with them. A library or recovery folder that you moved elsewhere in the settings is not touched. Documents such as `.ylp` files are never removed.

## Startup warning (SmartScreen)

Current distributions are not code-signed. If Windows SmartScreen displays “Windows protected your PC,” choose “More info” → “Run anyway” to launch.

## Privacy

The application does not send information over the network except to query GitHub for the latest version when you choose to check for updates (by enabling Check for Updates at Startup or selecting Check for Updates…). The query is an ordinary HTTPS request to fetch an update metadata file (with Use Beta Versions on, the beta update metadata file is fetched as well). Apart from the application name and version in the User-Agent, no information identifying the user is included. Live Link communicates only within the same machine.
