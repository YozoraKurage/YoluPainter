# Saving, Exporting and Importing

[日本語](../GUIDE_FILES.md)

This page covers saving a project (`.ylp`), the copy for distribution, recovery after a crash, exporting and importing PNG and PSD files, and the disk cache for large documents. Creating and opening projects is in [GUIDE_START.md](GUIDE_START.md).

## Save a project

Save to a `.ylp` with "Save" (`Ctrl+S`) and "Save As…" (`Ctrl+Shift+S`) in the File menu. The destination name must end in `.ylp` (any letter case), and the folder is created if it does not exist.

- Saving runs in a background thread, and the job card at the lower right shows "Saving" with its progress (it cannot be canceled). You can keep painting and looking while it runs, but what you paint after that is not in that save, and the document still shows as modified when it finishes.
- While saving, another save, Open, New and Save for Distribution are refused with a reason. If you close the window during a save, the app waits for the save to finish and closes after its result (after asking, if unsaved changes remain).
- When a save fails, nothing changes and the reason is shown. A save is committed by a single replacement from a verified temporary file, so the original file stays as it was if anything fails along the way. If another app changed the contents of the file after you opened it, the overwrite is refused (a changed modification time alone does not count). Do not edit the same file in two apps at once; save, then reopen it in the other.
- The version before an overwrite is kept in `<file name>-backups~/` in the same folder, newest first. "Backups to keep" in the "Files" category of Edit → "Settings…" chooses how many (0 to 1000; 0 keeps none) and only the older versions beyond it are deleted. "Keep all" never deletes any (the setting at first). A backup `.ylp` opens as a document without a save destination (the opened backup is never overwritten), so Save works as Save As; its model file is looked up from the folder of the original `.ylp`.

## Save for distribution

"Save for Distribution…" in the File menu writes a copy of the open project to a different file, leaving out what a `.ylp` can carry without its author noticing. The kinds that can be left out (the original PSD, unused assets, source paths of assets, model references, mesh maps, Unity material values, saved selections and so on) are listed in the window, and turning a switch off keeps that kind. Your working file does not change. What is removed, and compatibility, are described in [SAVE_FOR_DISTRIBUTION.md](../SAVE_FOR_DISTRIBUTION.md) (in Japanese).

## Do not lose work in a crash

While there are unsaved changes, the app writes recovery generations in the background. It writes:

- after a set time from when a change was found (15 seconds at first; from the previous checkpoint while you keep painting)
- when finished strokes have piled up (10 at first)
- when the window loses focus, and when the app closes

It does not take one in the middle of a stroke or a PSD import, but after it finishes. A failed checkpoint only reports the reason and does not stop painting. A generation has the same form as a `.ylp`, without composite PNGs and mesh maps.

After a crash, the Recovery window opens at the next start (you can open it any time from File → "Recovery…"). Choose "Open" or "Discard" from the list of generations (original name, number of sets, elapsed time). A recovered document opens as "Untitled (Recovered)" and is never written to the original `.ylp`; you choose where to save when you save.

In the window you can choose the checkpoint interval (10 seconds to 5 minutes), the generations kept (2 to 20), and the disk space (Automatic, Low, Standard, High; "Details" sets a size in GB). Recovery has a limit on the disk it uses, and the excess is removed from the oldest generation. A checkpoint is skipped when free space is low ([RECOVERY.md](../RECOVERY.md), in Japanese).

## Export PNG files

"Export Textures…" in the File menu opens the Export Textures window. Check the texture sets to export in the list on the left, decide the Output Path, Output Template and Padding on the right, check the names in Files to Export, then press "Export". The window closes when the writing starts. The Output Template you choose is kept in the settings and is the same the next time you open it.

- **Texture Sets**: The sets that can be exported are listed, all checked at first. A set you uncheck is not exported. The checks are kept until you open another project; a set you add comes in checked, and a set you rename shows its new name. A read-only set cannot be exported, so it has a lock icon and stays unchecked and cannot be touched. "Export" cannot be pressed while nothing is checked. When the Output Template is "Current Channel", only the current set is listed and it cannot be unchecked.
- **Output Path**: A file for "Current Channel" and a folder for the other templates; change it with "Choose…". Until you choose, it starts at the folder of the open project (for a model opened with Live Link, the folder Unity reported). A path you chose is kept until you open another project. "Export" cannot be pressed while no Output Path is decided or while an export is running.
- **Output Template**: Choose from "Current Channel", "Per Channel", "Unity Standard / URP Lit", "HDRP Lit" and "lilToon".
- **Padding**: How far colors are dilated beyond the UVs: "No padding", "Dilation n px" or "Dilation infinite". It is the same value as "Export padding" in the settings (changing either changes both). Nothing is dilated when there is no model.
- **Files to Export**: Lists, before writing, the name and color space (sRGB or Linear) of each file for the current Output Template, the checked sets and the Output Path. The names are decided the same way as the export decides them. A file that already exists in the Output Path has a dot at the left of its name. When something stops the export (nothing to write, names that would write the same file, and so on), the reason is shown in place of the list.

A file that already exists is confirmed before it is replaced (if you cancel, you are back in the window). If you cancel the export, or a failure happens before the last replacement begins, the original files are not changed.

### Export with a template

"Unity Standard / URP Lit", "HDRP Lit" and "lilToon" write the packed PNGs that shader reads, for every checked texture set, into the folder you choose. Names are `<name>[_<set name>]_<image>.png`, and the set name is added only when there are several sets (unchecking a set does not change the names of the others).

- A baked AO is used when it exists, and a stale AO is not.
- The effects shown on screen (filters, generators that read mesh maps, asset images) are included. When effects that are not working remain, for instance because the maps they read are missing, you are told that they are not in the exported images.
- Read-only sets are not exported.
- For a model opened with Live Link, the Output Path starts at the folder Unity reported.

### Export by channel

"Current Channel" writes the painting channel to one PNG, and "Per Channel" writes every channel in use, for the checked texture sets, into a folder. Values are the channel's composite as it is (not packed and not multiplied by color), and Normal alone follows the file Y direction in the Normal settings. Names are `<name>[_<set name>]_<channel>.png`.

## Import a PSD

"Import" in the File menu, then "PSD as a New Texture Set…" or "PSD into the Current Texture Set…", imports a PSD (RGB 8 bit). Dropping a .psd onto the window imports it the same way as a new texture set (when you drop several, only the first).

- A PSD is imported as a copy that is never written back to the original file. Raster layers, groups, solid fills, adjustments (invert, levels, hue/saturation, gradient map, tone curve, color balance, brightness/contrast, threshold, posterize), masks and clipping become layers. Layer locks come back as well.
- When something would be dropped or look different, such as layer effects, smart objects, text or unsupported adjustments, the "Import PSD Check" window lists it with layer names before importing. Nothing is imported until you press "Import".
- A PSB, anything other than RGB 8 bit, or a PSD over the budget is refused with a reason and nothing changes. The size limits come from the "Layer memory" budget in the settings.
- The imported document's composite is compared with the PSD's merged image, and a large difference is shown in the window.

How things are sorted, and the limits, are in [PSD.md](../PSD.md) (in Japanese).

## Export a PSD

"Export PSD…" in the File menu opens the "Export PSD" window. Choose the mode and the channels, then press "Export…".

- The mode is "Bake and write" (keeps the layers, and writes filters, generators, images, paths and other features PSD has no form for as evaluated pixels) or "Flatten to one layer" (writes only the composite, as a single layer).
- Channels start with Color only. If you choose several, one `name_channel.psd` is written per channel.
- When something will be baked, rounded or dropped, the "Export PSD Check" window lists it with layer names before writing. Nothing is written until you press "Write".
- The export is of a copy of the document and the document does not change. Effects stay in the document, but baked layers carry no effect settings, so importing the PSD again does not bring the effects back.

Opening the file in Photoshop or CLIP STUDIO PAINT can re-evaluate the adjustment layers and change how it looks. How it is written, and the limits, are in [PSD.md](../PSD.md) (in Japanese).

## Import PNG files

PNG files can be imported in these places:

- Stencils (the Stencil tab in Properties)
- Asset images: through "Import from File…" in the image list of a fill layer, or "Use in this project" in the library. Fill layers, Image generators and path ribbons use them ([GUIDE_FILL.md](GUIDE_FILL.md))
- The library: "Add files to the library…" or dropping onto the grid

A smart material's `.ylsmart` can be imported into the assets and the library.

## Work with large documents

"Disk cache" in the settings (on at first) moves tiles beyond the combined "Layer memory" and "Undo history" budgets, least recently used first, into a cache file, so you can keep painting. When it is off, edits beyond the budgets are refused.

"Details" opens "Cache limit" (automatic is 64 GiB or half the free space of the folder, whichever is smaller) and "Cache folder" (the system temporary folder at first; a faster drive brings tiles back faster). When the limit is used up, edits beyond it are refused and the oldest undo steps are dropped.

- Tiles are written uncompressed, so the disk used never exceeds the amount of the document's pixels. Cache files left by a previous run are deleted in the background at start.
- If a tile cannot be read back from the disk, that texture set becomes read-only and you are told. A save writes what was there when the set was opened. A set that was never saved has no original content, so only that set is left out of saving and recovery checkpoints.

## Where the file windows start

A window that chooses a file or folder starts in the first existing place of the following.

- **Open, import, export**: the folder you last chose for that kind (project, model, PSD import and export, image import and export, brushes, fonts, key settings, assets and library, color sets, Save for Distribution, settings folders) → the folder of the open project's `.ylp` → the OS Documents folder.
- **Save, Save As**: for a document opened from a backup `.ylp`, the folder of the original `.ylp` → the folder of the open project's `.ylp` → the folder last chosen for projects → the OS Documents folder.

The folder last chosen is kept per kind in `places.conf` in the settings folder (if you delete it, the app remembers again from the next choice). A folder that no longer exists is not used. When you choose in the Export Textures window, an Output Path that is already decided comes first (the one you chose; otherwise, for a model opened with Live Link, the folder Unity reported; otherwise the project's folder). Only when none is decided, as in a document you have not saved, does the window start in the order above.

## Handing files to older versions and Unity

A `.ylp` that uses any of these is saved under a newer version number that the 0.4.x standalone app and the Unity version up to 0.4.x cannot open (the Unity bridge does not open `.ylp` from 0.5.0):

- Text layers
- Two or more paths, path types, tips, symmetry, corners or handles, or paths on fill layers ([GUIDE_PATHS.md](GUIDE_PATHS.md))
- Point gradients, an image with anisotropic filtering turned off, 0.5.0 effects, or "Across UV seams" turned off ([GUIDE_FILL.md](GUIDE_FILL.md))
- A bake priority changed from the default ([GUIDE_3D.md](GUIDE_3D.md))

A document that uses none of them keeps the earlier version. To go back to the Unity version up to 0.4.x, remove the feature and save again.

A `.ylp` that uses any of these cannot be opened by the standalone app up to 0.5.x either: paths whose brush anti-aliasing is not "None" (a new path takes the anti-aliasing of the current brush, which is Medium by default; see [BRUSH.md](../BRUSH.md), in Japanese), rulers (on layers or groups, symmetry rulers too), or paths with a diagonal line symmetry or a line symmetry of 6 or more lines ([GUIDE_PAINT.md](GUIDE_PAINT.md)). It is refused by the version number alone, without touching the file. Set the anti-aliasing of each path to "None" and save again, and the paths keep the earlier version. Turn the path symmetry off, or set it again from a symmetry ruler with 2 or 4 lines in the vertical or horizontal direction, then delete all rulers and save again, and it opens. Which version reads how much is in [UNITY.md](UNITY.md) and [YLP_FORMAT.md](../YLP_FORMAT.md) (in Japanese).
