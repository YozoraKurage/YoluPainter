# Settings

[日本語](../GUIDE_SETTINGS.md)

This page explains each item in Edit → Settings… (`Ctrl+,`), section by section. The window has five sections: General, Memory, Processing, 3D View and Files. You can move it by dragging its title. If the screen is too short to fit it, scroll inside the window to reach the last row.

Values are written to `YoluPainter/settings.conf` in the settings folder (`%APPDATA%` on Windows, `~/.config` on Linux), and only the values that differ from the defaults. A value that is not valid is reset to the default for that item only, and you are told why.

## General

| Item | What it does |
|---|---|
| Language | Japanese or English. The View menu also switches it |
| Accept Live Link from Unity | When on, opens the model and materials sent with Open in YoluPainter in the Unity Editor. When the app is launched with `--livelink`, it accepts them regardless of this setting ([UNITY.md](UNITY.md)) |
| Save material values received from Unity | Stores the lilToon values received through Live Link in the `.ylp` (not the pixels of textures; they are read from their files on reopening). When off, they are removed on the next save |
| Accept external commands | See “External commands (MCP)” below |
| Export padding | How far the colors at the UV edges are spread into texels no UV touches when exporting. Choose Off, 2, 4, 8, 16, 32, 64 texels or Fill (all the way) (the default is Fill). It keeps mipmaps and filtering from pulling in other colors |

### Language

When the settings have no language yet (such as the first start), the app starts in the OS language: Japanese if that is Japanese, English otherwise. The OS language is read from the display language on Windows, and from `LC_ALL`, `LC_MESSAGES` and `LANG` on Linux. The environment variable `YOLUPAINTER_LANG` (`ja` or `en`) takes precedence. Once a language has been saved to the settings, the app always starts in it.

### External commands (MCP)

Only while Accept external commands is on, the app listens at `http://127.0.0.1:<port>/mcp` for commands from AI assistants (MCP) and the command line. It is off by default. Turning it on shows a Port row (17347 by default; 1024 to 65535). When you change it, use the same port in the programs that connect. A small dot appears at the right end of the status bar, and its color shows whether the app is waiting, connected or unable to listen (the tooltip has the URL or the reason).

- It is communication inside this PC only. There is no password, so while it is on, programs of other accounts on this PC can connect too.
- Each command becomes one undo step. Edits while you are painting or while a save is in progress, and edits to a read-only texture set, are refused with a reason. Commands that open a different document are not accepted.
- Turning it off stops listening and closes the connections.

For connecting, see [MCP.md](MCP.md); for the command line, see [CLI.md](CLI.md).

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

Open Details to see two more items.

- Cache limit: how much disk space the cache may use. Auto is 64 GiB or half the free space of the folder, whichever is smaller. When it is full, edits beyond it are refused.
- Cache folder: choose the folder for the cache file with Choose… (a faster drive brings moved tiles back faster). Default goes back to the system temporary folder.

## Processing

| Item | What it does |
|---|---|
| CPU threads | The most threads the CPU work (compositing, brushes, fills, selections) uses at once. Choose Automatic, 1, powers of 2 or the number of logical processors. Any value gives the same pixels; fewer are slower on large canvases but leave cores to other programs. It takes effect from the next start, and “applies after restart” is shown after you change it |
| Display compositing | Where the layers shown on the 2D Canvas are composited: Automatic, GPU or CPU |
| GPU memory | How much GPU memory the 3D view, the canvas compositing and the asset previews may use: Automatic, Low, Standard or High |

**Display compositing** affects only what is shown on screen. Saving, exporting and the 3D view always composite on the CPU, whatever you choose (GPU pixels can differ from the CPU result by a small amount, within 2 even for documents with many levels; this is a display-only difference). Automatic composites on the GPU when a usable GPU exists, and on the CPU for a software GPU. Documents with adjustment layers, isolated groups, Normal channels and effects are composited on the GPU too (the effects are computed on the CPU, and the resulting pixels are uploaded to the GPU). A document falls back to the CPU when its drawn tiles exceed the canvas budget given by GPU memory (512 MiB at Standard), when it is larger than the device's texture limit, or when its groups are nested too deeply. The environment variable `YOLUPAINTER_CANVAS` (`auto`, `gpu` or `cpu`) can force it.

When **GPU memory** runs short, the 3D view drops the other sets' pictures and shows the current one smaller (the texture and exports are unchanged). Automatic follows the GPU's memory only when it is known (Low when there is little, a larger Standard when there is plenty); otherwise it is Standard. Open Details to get a Total slider, which is split 4 : 4 : 1 between the 3D pictures, the canvas compositing and the asset previews. Moving it replaces the level with a custom amount.

## 3D View

| Item | What it does |
|---|---|
| Orbit center | The pivot when orbiting the 3D view: View center, Surface (auto depth), Model center or Texture set center |
| Zoom center | The center when zooming the 3D view: Toward view center or Toward pointer |
| UV Wireframe | Two color swatches: the UV wireframe color and the Overlapping UV color. Pressing one opens the color window, where you set the color and opacity |

The orbit and zoom centers are the same values as Navigation in the 3D view's display settings; changing either changes the same setting ([GUIDE_3D.md](GUIDE_3D.md)).

## Files

| Item | What it does |
|---|---|
| Library folder | Where your own library (the folder for your assets) lives. The default is `YoluPainter/Library` in the settings folder. Choose it with Choose…, and restore the default with Default. Only an absolute path is accepted |
| Backups to keep | How many replaced versions to keep per file (0 to 1000), newest first, when you overwrite a save. They are kept in the `<file name>-backups~` folder next to the file. Only the older ones beyond this are deleted. 0 makes no backups (existing ones are not deleted) |
| Keep all | When on, backups are never deleted. It is on by default |

## Settings outside this window

- **Pen pressure**: View → Pen Pressure… sets the low limit, high limit and curve ([GUIDE_PAINT.md](GUIDE_PAINT.md)).
- **Recovery**: the Recovery window (File → Recovery…) sets the checkpoint interval, the generations kept and the disk space used ([RECOVERY.md](../RECOVERY.md), in Japanese).
- **Updates**: Help → Check for Updates…, Check for Updates at Startup and Use Beta Versions ([INSTALL.md](INSTALL.md)).
- **The selection button bar**: Show the Selection Button Bar in the Select menu ([GUIDE_SELECT.md](GUIDE_SELECT.md)).
- **Panel layout**: remembered in `YoluPainter/layout.json` in the settings folder ([GUIDE_START.md](GUIDE_START.md)).
