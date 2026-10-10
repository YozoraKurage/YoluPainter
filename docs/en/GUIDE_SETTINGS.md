# Settings

[日本語](../GUIDE_SETTINGS.md)

This page explains each item in Edit → Settings… (`Ctrl+,`), category by category. The categories are listed on the left; pressing one shows its items on the right. They are General, Pen, Keyboard Shortcuts, Display, Memory, Processing, 3D View, Files, Live Link & Commands and Updates (Updates appears only in builds that can check for updates). The window remembers the category you last opened and starts there the next time. The right side scrolls separately for each category. You can move the window by dragging its title.

Type in the Search Settings field at the top, and only the items whose names (in the language of the screen) match are listed on the right, from all categories and grouped by category. On the left, the categories that have a match light up. When the text matches the name of a category, all the items of that category are listed. Pressing a category empties the search field and opens that category. `Esc` empties the search field when it has text, and closes the window when it does not. It does not close the window when something else uses the `Esc`: a popup, an input field such as the port or the search field of the shortcut table, waiting for a key to assign, or the color window. Closing empties the search field, and the next opening starts from the category you last opened. When the categories on the left do not fit in a low screen, the column scrolls.

Values are written to `YoluPainter/settings.conf` in the settings folder (`%APPDATA%` on Windows, `~/.config` on Linux), and only the values that differ from the defaults. A value that is not valid is reset to the default for that item only, and you are told why.

## General

| Item | What it does |
|---|---|
| Language | Japanese or English. The screen changes as soon as you choose |

### Language

When the settings have no language yet (such as the first start), the app starts in the OS language: Japanese if that is Japanese, English otherwise. The OS language is read from the display language on Windows, and from `LC_ALL`, `LC_MESSAGES` and `LANG` on Linux. The environment variable `YOLUPAINTER_LANG` (`ja` or `en`) takes precedence. Once a language has been saved to the settings, the app always starts in it.

## Pen

| Item | What it does |
|---|---|
| Tablet pressure (experimental) (Mac) | Reads the pressure, tilt, eraser end and side buttons that tablet drivers such as Wacom and XP-Pen send as standard macOS events. On by default. When off, the pen draws like a mouse. The change takes effect immediately |
| Pen input (Windows) | How the pen's pressure, tilt, rotation, eraser end and side buttons are read: Windows Ink (the default) or WinTab (the input that tablet drivers such as Wacom's provide). The change takes effect immediately and applies to separate windows too. A pen that is touching is treated as lifted when you switch |
| Pen Pressure | Adjusts the pen's pressure before it is passed to the brush. A curve, Low, High, a drawing frame, and the Auto, Clear, Revert and Default buttons |

If you choose WinTab but the WinTab driver (`Wintab32.dll`) is missing, does not respond, no tablet is connected, or WinTab cannot be opened, the pen is read through Windows Ink and the reason is shown once. The choice stays WinTab, and it is tried again when the window is brought to the front. When WinTab's position does not match the cursor (for example in the driver's mouse mode), the points are placed at the cursor position (pressure and the rest still come from WinTab).

### Pen Pressure

When the pen's pressure is too strong or too weak, this sets how the pen pressure (across the curve) becomes the pressure the brush gets (up the curve). The mouse's pressure stays 1 and does not go through the adjustment.

- Curve: click an empty spot to add a point, drag to move one, right-click to remove one. `Esc` cancels a drag. The faint bars show the pressure of the strokes you drew.
- Low and High: pressure at or below Low counts as 0, and pressure at or above High counts as 1.
- Drawing frame: draw a few strokes at your usual strength. The strokes are kept only inside the frame and are not part of the document. They are discarded when you leave Pen or close the Settings window.
- Auto: works out Low, High and the curve from the pressure of the strokes you drew. When there are not enough strokes, or the pressure barely varies, it does not decide and shows a short reason. Clear removes the drawn strokes. Revert goes back to the adjustment from when Pen was opened, and Default goes back to Low 0%, High 100% and a straight line.

The adjustment is stored in the settings file (not in documents or brushes) and applies on the 2D Canvas and in the 3D View alike. When a finger or pen sends a force as touch input, that force goes through the same adjustment too ([GUIDE_PAINT.md](GUIDE_PAINT.md)).

## Keyboard Shortcuts

View and change the keyboard and mouse combinations and the pie menus. Press Keyboard Shortcuts on the left, and its sections (Everywhere & View / Paint / Edit / Pose / During an Operation / Pie Menus) appear below it, with the table (each action's name, keys and mouse combination) on the right. Above the table are a field to search by name and Find by Key (these two narrow only the table), and below it are Reset to Default, Export… and Import…. How to use it is in "Change shortcuts" in [GUIDE_KEYS.md](GUIDE_KEYS.md).

## Display

| Item | What it does |
|---|---|
| Display compositing | Where the layers shown on the 2D Canvas are composited: Automatic, GPU or CPU |
| VSync | Whether the screen update waits for the monitor's vertical sync. Off by default (it does not wait). When on, there is no screen tearing, but the line you draw with the pen takes longer to appear. It takes effect from the next start, and “applies after restart” is shown after you change it |
| GPU memory | How much GPU memory the 3D view, the canvas compositing and the asset previews may use: Automatic, Low, Standard or High |
| Use RT cores for baking | Whether mesh-map baking uses the GPU's RT cores (hardware ray tracing). On by default. On a GPU without them, or when the answers do not agree with the path that does not use them, baking goes without RT cores whatever this is set to. Turn it off on a PC whose driver hangs: baking then always goes without RT cores. How the screen is drawn does not change |

**Display compositing** affects only what is shown on screen. Saving, exporting and the 3D view always composite on the CPU, whatever you choose (GPU pixels can differ from the CPU result by a small amount, within 2 even for documents with many levels; this is a display-only difference). Automatic composites on the GPU when a usable GPU exists, and on the CPU for a software GPU. Documents with adjustment layers, isolated groups, Normal channels and effects are composited on the GPU too (the effects are computed on the CPU, and the resulting pixels are uploaded to the GPU). A document falls back to the CPU when its drawn tiles exceed the canvas budget given by GPU memory (512 MiB at Standard), when it is larger than the GPU's texture limit, or when its groups are nested too deeply. The environment variable `YOLUPAINTER_CANVAS` (`auto`, `gpu` or `cpu`) can force it.

While **VSync** is off (the default), presenting to the screen does not wait for the vertical sync. The delay between the pen input and the line appearing on screen gets shorter, at the cost of possible screen tearing. If the graphics environment has no way of presenting without waiting, it waits anyway. So that repaint requests made every frame do not use up the CPU and GPU, the app puts a lower limit on the interval between repaints (no sooner than 1/240 second when there is input; when there are only repaint requests, no sooner than one refresh of the monitor the window is on, where the refresh rate is read on Windows only and kept within 60 to 240 Hz, and 120 Hz when it cannot be read). While it is on, the vertical sync sets the interval, so no limit is applied. It is decided when the window is created, so a change takes effect from the next start.

**Use RT cores for baking** can also be turned off with the environment variable `YOLUPAINTER_BAKE_RAY_QUERY=0` (`false`, `off` and `no` work too; while the variable says off, the setting stays off even when it is on, and the row cannot be pressed). Changing it recreates the GPU device at the next check or bake.

When **GPU memory** runs short, the 3D view drops the other sets' pictures and shows the current one smaller (the texture and exports are unchanged). Automatic follows the GPU's memory only when it is known (Low when there is little, a larger Standard when there is plenty); otherwise it is Standard. Open Details at the right of the row to get a Total slider, which is split 4 : 4 : 1 between the 3D pictures (including the selection edge and overlays in the 3D view), the canvas compositing and the asset previews. Moving it replaces the level with a custom amount.

## Memory

| Item | What it does |
|---|---|
| Undo history | Memory for the undo history, in total over all texture sets. The oldest steps beyond it are dropped |
| Layer memory | The most memory all layers in all texture sets may use for pixels, in total. An edit that would exceed it is refused without changing anything |
| One operation | Undo data one stroke or fill may keep; also the working memory of exports. A bigger one is stopped without changing anything |
| Minimum undo steps | The newest steps kept even beyond the undo budget (0 to 100). 0 makes the budget strict |
| Disk cache | See “Disk cache” below |

Choose each of the three memory budgets as Auto or a number of MiB. Auto is derived from physical memory (half for layer memory, 1/8 for undo history, 1/16 for one operation, each with a minimum and a cap). Each is a ceiling, and nothing is reserved up front. Undo history and layer memory are totals for the whole project. One operation and Minimum undo steps are per texture set. Opening a `.ylp` reads pixels up to the Layer memory setting, with a minimum of 256 MiB.

### Disk cache

Disk cache moves the tiles beyond the memory limit (the sum of the layer memory and undo history budgets) to disk, least recently used first, so you can keep painting on a large document. It is on by default. When off, edits beyond the budgets are refused. If a tile cannot be read back from disk, the app makes that texture set read-only and tells you. For more, see [GUIDE_FILES.md](GUIDE_FILES.md).

Open Details at the right of the row to see two more items.

- Cache limit: how much disk space the cache may use. Auto is 64 GiB or half the free space of the folder, whichever is smaller. When it is full, edits beyond it are refused.
- Cache folder: choose the folder for the cache file with Choose… (a faster drive brings moved tiles back faster). Default goes back to the system temporary folder.

## Processing

| Item | What it does |
|---|---|
| CPU threads | The most threads the CPU work (compositing, brushes, fills, selections) uses at once. Choose Automatic, 1, powers of 2 or the number of logical processors. Any value gives the same pixels; fewer are slower on large canvases but leave cores to other programs. It takes effect from the next start, and “applies after restart” is shown after you change it |

## 3D View

| Item | What it does |
|---|---|
| Orbit center | The pivot when orbiting the 3D view: View center, Surface (auto depth), Model center or Texture set center |
| Zoom center | The center when zooming the 3D view: Toward view center or Toward pointer |
| Orthographic on axis views | Switches to orthographic when you pick an axis view or snap-orbit onto an axis, and back to perspective when you orbit off the axis. On by default ("Axis views and orthographic" in [GUIDE_3D.md](GUIDE_3D.md)) |
| UV Wireframe | Two color swatches: the UV wireframe color and the Overlapping UV color. Pressing one opens the color window, where you set the color and opacity |

The orbit and zoom centers are the same values as Navigation in the 3D view's display settings; changing either changes the same setting ([GUIDE_3D.md](GUIDE_3D.md)).

## Files

| Item | What it does |
|---|---|
| Export padding | How far the colors at the UV edges are spread into texels no UV touches when exporting. Choose No padding, Dilation 2 px, 4 px, 8 px, 16 px, 32 px, 64 px or Dilation infinite (the default is Dilation infinite). It keeps mipmaps and filtering from pulling in other colors. It is the same value as "Padding" in the File → Export Textures… window |
| Library folder | Where your own library (the folder for your assets) lives. The default is `YoluPainter/Library` in the settings folder. Choose it with Choose…, and restore the default with Default. Only an absolute path is accepted |
| Backups to keep | How many replaced versions to keep per file (0 to 1000), newest first, when you overwrite a save. They are kept in the `<file name>-backups~` folder next to the file. Only the older ones beyond this are deleted. 0 makes no backups (existing ones are not deleted) |
| Keep all | When on, backups are never deleted. It is on by default |

## Live Link & Commands

| Item | What it does |
|---|---|
| Accept Live Link from Unity | When on, opens the model and materials sent with Open in YoluPainter in the Unity Editor. When the app is launched with `--livelink`, it accepts them regardless of this setting ([UNITY.md](UNITY.md)) |
| Save material values received from Unity | Stores the lilToon values received through Live Link in the `.ylp` (not the pixels of textures; they are read from their files on reopening). When off, they are removed on the next save |
| Accept external commands | See “External commands (MCP)” below |

### External commands (MCP)

Only while Accept external commands is on, the app listens at `http://127.0.0.1:<port>/mcp` for commands from AI assistants (MCP) and the command line. It is off by default. Turning it on shows a Port row (17347 by default; 1024 to 65535). When you change it, use the same port in the programs that connect. A small dot appears at the right end of the status bar, and its color shows whether the app is waiting, connected or unable to listen (the tooltip has the URL or the reason).

- It is communication inside this PC only. There is no password, so while it is on, programs of other accounts on this PC can connect too.
- Each command becomes one undo step. Edits while you are painting or while a save is in progress, and edits to a read-only texture set, are refused with a reason. Commands that open a different document are not accepted.
- Turning it off stops listening and closes the connections.

For connecting, see [MCP.md](MCP.md); for the command line, see [CLI.md](CLI.md).

## Updates

This appears only in builds that can check for updates (an app built from source has neither this category nor Check for Updates… in the Help menu).

| Item | What it does |
|---|---|
| Check for Updates at Startup | When on, the app asks GitHub for the latest version on each launch. The choice you were asked at the first launch can be changed here too ([INSTALL.md](INSTALL.md)) |
| Use Beta Versions | Also offers beta versions as updates. When off, only stable releases are offered. It is off by default |

To check by hand, use Help → Check for Updates….

## Settings outside this window

- **Recovery**: the Recovery window (File → Recovery…) sets the checkpoint interval, the generations kept and the disk space used ([RECOVERY.md](../RECOVERY.md), in Japanese).
- **The selection button bar**: Show the Selection Button Bar in the Select menu ([GUIDE_SELECT.md](GUIDE_SELECT.md)).
- **Panel layout**: remembered in `YoluPainter/layout.json` in the settings folder ([GUIDE_START.md](GUIDE_START.md)).
- **Key and mouse assignments and pie menus**: changed in the Keyboard Shortcuts category; only what differs from the defaults is remembered in `YoluPainter/keymap.json` in the settings folder. How a tool key behaves is written under `tool_keys` as the ID of the action that selects the tool and the behavior (`hold` for only while held, `tap_or_hold` for tap to switch and hold for temporary; for example `"tool_keys": { "tool.eraser": "hold" }`. The default, switch on press, is not written) ([GUIDE_KEYS.md](GUIDE_KEYS.md)).
