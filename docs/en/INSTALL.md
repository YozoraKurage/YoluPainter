# Download and updates

[日本語](../INSTALL.md)

Download from [Releases](https://github.com/YozoraKurage/YoluPainter/releases). Windows (64-bit) has two distribution formats, an installer and a zip. An extension for Claude Desktop (`.mcpb`) is attached too. Mac has an experimental zip ([below](#mac-experimental)).

- `yolupainter-<version>-x86_64-pc-windows-msvc-setup.exe` (installer): installs per user without administrator privileges. The default destination is `%LOCALAPPDATA%\Programs\YoluPainter` (changeable), with a Start menu entry and optional `.ylp` file association. 
  - Uninstall through Settings → Apps; you will be asked whether to remove settings and recovery data as well (they remain if no prompt is shown). Things you made are removed in neither case ([table below](#uninstalling-and-your-data)).
  - Use `/S` for silent installation, `/ASSOC=1` to enable file association (`/ASSOC=0` to disable it), and `/RUN` to launch the application after installation.
  - For a silent uninstall use `/S`, and add `/DELETEDATA` to remove settings and recovery data as well.
- `yolupainter-<version>-x86_64-pc-windows-msvc.zip`: extract and run `yolupainter.exe`; no installation is needed.
- `yolupainter-<version>-x86_64-pc-windows-msvc.mcpb`: an extension for Claude Desktop ([Operating from an AI assistant](MCP.md)). Double-click it to install. It only relays to the running app, and the tools are answered by the app (they follow app updates). Claude Code and Codex use the plugin instead.

Both include the command-line program `yolupainter-cli.exe` ([usage](CLI.md)), `README.md` and a `docs` folder (English under `docs/en`) with this document, so they can be read offline.

## Mac (experimental)

- `yolupainter-<version>-macos-universal-experimental.zip`: the folder it unzips to holds `YoluPainter.app` (a universal build with code for both Apple Silicon and Intel), `README.md`, the full license texts, and a `docs` folder with this document (English under `docs/en`). Move `YoluPainter.app` to Applications or elsewhere and use it from there. The command-line program ([usage](CLI.md)) is `YoluPainter.app/Contents/MacOS/yolupainter-cli`.

This is an experimental build with no Apple Developer signature or notarization (only an ad-hoc signature). The executables require macOS 11 or later, and on an Apple Silicon Mac (macOS 27), only launching, writing settings and logs, the signature and the universal structure have been checked. The window display and drawing have not been checked, and it has not been run on an Intel Mac. Because it is unsigned, macOS blocks `YoluPainter.app` the first time you open it after unzipping. To open it, follow these steps ([Apple's guide](https://support.apple.com/guide/mac-help/open-a-mac-app-from-an-unidentified-developer-mh40616/mac)):

1. Double-click `YoluPainter.app`. It is blocked; close the message.
2. Choose Apple menu > System Settings, then click Privacy & Security in the sidebar.
3. Go to the Security section, then click Open.
4. Click Open Anyway (it is available for about an hour after you try to open the app).
5. Enter your login password, then click OK. From then on, double-click opens it.

The Mac app does not update itself (it neither downloads nor replaces anything). If you answer Yes to the first question, it asks GitHub for the latest version at each launch (or by hand with Help → Check for Updates…); when a new version exists, the Help heading shows an indicator and “Open the YoluPainter x.y.z release” opens that version's release page. Unzip the new zip and replace `YoluPainter.app`; your settings and projects are kept.

To uninstall, move `YoluPainter.app` to the Trash. Other data is not removed automatically; delete it yourself if you do not need it.

| Location | Contents |
| --- | --- |
| `~/Library/Application Support/YoluPainter/` | Settings, recovery generations (`recovery/`), crash records (`logs/`), the Live Link exchange folder (`LiveLink/`), and things you made such as libraries and brushes |
| `~/Library/Caches/YoluPainter/thumbnails/` | Thumbnail cache |

## Updates

On first launch, the application asks whether to check for updates at startup. Only choosing Yes enables a request to GitHub for the latest version on each launch. Change this at any time with Check for Updates at Startup in the Updates category of Edit → Settings…, or check manually through Help → Check for Updates….

Turning on Use Beta Versions in the Updates category of Edit → Settings… also offers beta versions (versions with an `alpha`, `beta` or `rc` identifier, such as `0.4.0-rc.1`) and recommends the newer of the stable release and the beta. It is off by default, and while off only stable releases are considered. While you run a beta, the status bar shows “Beta” to the left of the version. The app never updates to an older version, so after turning the setting off, nothing is offered until the next stable release is newer than the version you run.

When a new version is available, the Help heading shows an indicator. On Windows installed through the installer, “Update to YoluPainter x.y.z” downloads, verifies, and installs the update, asking you to save first if there are unsaved changes. Downloads are used only after verifying the signed update metadata's signature, size, and SHA-256. Portable Windows ZIP distributions, Linux and Mac open the release page instead. Applications built from source do not have update items (Check for Updates… in the Help menu and Updates in Settings); only distribution builds embed the update public key.

When another window of the installed YoluPainter is open, the update does not start and the app says “Another YoluPainter is running” (the installer cannot replace an executable that is in use). The downloaded installer is kept: close the other windows and press “Update and restart” again to install it right away. In a silent run the installer waits up to 60 seconds for the executable to be released; if it is still in use, it exits without changing anything. The application's own update runs the installer with `/RUN`, and then the application that is already installed is started again (a silent run without `/RUN` does not start it).

## Uninstalling and your data

Uninstalling removes the installed files (the application, documents, shortcut and file association) and the installers downloaded for updates (`%LOCALAPPDATA%\YoluPainter\updates`). Other data is removed only when you answer Yes to “Also delete settings and recovery data?” (or pass `/DELETEDATA` to a silent uninstall), and then only what the table below marks as removed. Things you made are never removed, whichever answer you give.

| Location | Contents | On “delete” |
| --- | --- | --- |
| `%APPDATA%\YoluPainter\settings.conf` | Settings | Removed |
| `%APPDATA%\YoluPainter\recovery.conf`, `update.conf`, `layout.json` | Recovery settings, the update-check and beta choices, panel and window layout | Removed |
| `%APPDATA%\YoluPainter\places.conf` | Where the file windows start (the folder last chosen for each kind) | Removed |
| `%APPDATA%\YoluPainter\recovery\` | Recovery generations | Removed |
| `%APPDATA%\YoluPainter\logs\` | Crash records | Removed |
| `%LOCALAPPDATA%\YoluPainter\thumbnails\` | Thumbnail cache | Removed |
| `%LOCALAPPDATA%\YoluPainter\LiveLink\` | The Live Link exchange folder (requests and replies with Unity, the awake mark) | Removed |
| `%APPDATA%\YoluPainter\Library\` (the default library folder) | Personal library | Kept |
| `%APPDATA%\YoluPainter\brushes\` | Your brushes and erasers | Kept |
| `%APPDATA%\YoluPainter\subtools\` | Your sub-tools | Kept |
| `%APPDATA%\YoluPainter\gradients\` | Gradient sets | Kept |
| `%APPDATA%\YoluPainter\colorsets\` | Color sets | Kept |
| `%APPDATA%\YoluPainter\hide_presets\` | Presets for hiding parts of a model | Kept |
| `%APPDATA%\YoluPainter\pose_presets\` | Pose presets | Kept |
| `%APPDATA%\YoluPainter\actions\` | Actions | Kept |
| `%APPDATA%\YoluPainter\keymap.json` | Your assignments for keyboard shortcuts, mouse combinations and pie menus (an unreadable file is moved to `keymap.broken.json` the next time it is written; if that name is taken, a number is added, as in `keymap.broken-2.json`) | Kept |
| `%APPDATA%\YoluPainter\tools.json` | The order of the toolbar and the brush groups (an unreadable file is moved to `tools.broken.json`) | Kept |

When kept items, or files this application did not create, are present, the `%APPDATA%\YoluPainter` folder stays with them. A library or recovery folder that you moved elsewhere in the settings is not touched. Documents such as `.ylp` files are never removed.

## Startup warning (SmartScreen)

Current distributions are not code-signed. If Windows SmartScreen displays “Windows protected your PC,” choose “More info” → “Run anyway” to launch.

## Privacy

The application does not send information over the network except to query GitHub for the latest version when you choose to check for updates (by enabling Check for Updates at Startup or selecting Check for Updates…). The query is an ordinary HTTPS request to fetch an update metadata file (with Use Beta Versions on, the beta update metadata file is fetched as well). Apart from the application name and version in the User-Agent, no information identifying the user is included. Live Link only exchanges files in a folder on the same machine, and external operation listens on `127.0.0.1` (this machine only), and only while it is turned on in the settings.
