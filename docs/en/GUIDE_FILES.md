# Saving, Exporting and Importing

[日本語](../GUIDE_FILES.md)

This page covers saving a project (`.ylp`), the copy for distribution, recovery after a crash, exporting and importing PNG and PSD files, and the disk cache for large documents. Creating and opening projects is in [GUIDE_START.md](GUIDE_START.md).

## Save a project

Save to a `.ylp` with "Save" (`Ctrl+S`) and "Save As…" (`Ctrl+Shift+S`) in the File menu. The destination name must end in `.ylp` (any letter case), and the folder is created if it does not exist.

- Saving runs in a background thread, and the job card at the lower right shows "Saving" with its progress (it cannot be canceled). You can keep painting and looking while it runs, but what you paint after that is not in that save, and the document still shows as modified when it finishes.
- While saving, another save, Open, New and Save for Distribution are refused with a reason. If you close the window during a save, the app waits for the save to finish and closes after its result (after asking, if unsaved changes remain).
- When a save fails, nothing changes and the reason is shown. A save is committed by a single replacement from a verified temporary file, so the original file stays as it was if anything fails along the way. If another app changed the contents of the file after you opened it, the overwrite is refused (a changed modification time alone does not count). Do not edit the same file in two apps at once; save, then reopen it in the other.
- The version before an overwrite is kept in `<file name>-backups~/` in the same folder, newest first. "Backups to keep" in the "Files" section of Edit → "Settings…" chooses how many (0 to 1000; 0 keeps none) and only the older versions beyond it are deleted. "Keep all" never deletes any (the setting at first). When you open a backup `.ylp`, the model file is looked up from the folder of the original `.ylp`.

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

Exports are listed under the "Export" heading in the File menu. A file that already exists in the destination folder is confirmed before it is replaced. If you cancel, or a failure happens before the last replacement begins, the original files are not changed. The width that colors are dilated beyond the UVs is "Export padding" in the settings; nothing is dilated when there is no model.

### Export with a template

"Template: Unity Standard / URP Lit…", "Template: HDRP Lit…" and "Template: lilToon…" write the packed PNGs that shader reads, for every texture set, into the folder you choose. Names are `<name>[_<set name>]_<image>.png`, and the set name is added only when there are several sets.

- A baked AO is used when it exists, and a stale AO is not.
- The effects shown on screen (filters, generators that read mesh maps, asset images) are included. When effects that are not working remain, for instance because the maps they read are missing, you are told that they are not in the exported images.
- Read-only sets are not exported.
- For a model opened with Live Link, the default destination is the folder Unity reported.

### Export by channel

"Channel as PNG…" writes the painting channel to one PNG, and "All Channels as Images…" writes every channel in use, for all texture sets, into a folder. Values are the channel's composite as it is (not packed and not multiplied by color), and Normal alone follows the file Y direction in the Normal settings. Names are `<name>[_<set name>]_<channel>.png`.

## Import a PSD

"PSD as a New Texture Set…" or "PSD as the Current Set's Document…" in the File menu imports a PSD (RGB 8 bit). Dropping a .psd onto the window imports it the same way as a new texture set (when you drop several, only the first).

- A PSD is imported as a copy that is never written back to the original file. Raster layers, groups, solid fills, adjustments (invert, levels, hue/saturation, gradient map, tone curve, color balance, brightness/contrast, threshold, posterize), masks and clipping become layers. Layer locks come back as well.
- When something would be dropped or look different, such as layer effects, smart objects, text or unsupported adjustments, the "Import PSD Check" window lists it with layer names before importing. Nothing is imported until you press "Import".
- A PSB, anything other than RGB 8 bit, or a PSD over the budget is refused with a reason and nothing changes. The size limits come from the "Layer memory" budget in the settings.
- The imported document's composite is compared with the PSD's merged image, and a large difference is shown in the window.

How things are sorted, and the limits, are in [PSD.md](../PSD.md) (in Japanese).

## Export a PSD

"PSD…" in the File menu opens the "Export PSD" window. Choose the mode and the channels, then press "Export…".

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

## Handing files to older versions and Unity

A `.ylp` that uses any of these is saved under a newer version number that the 0.4.x standalone app and the Unity version up to 0.4.x cannot open (the Unity bridge does not open `.ylp` from 0.5.0):

- Text layers
- Two or more paths, path types, tips, symmetry, corners or handles, or paths on fill layers ([GUIDE_PATHS.md](GUIDE_PATHS.md))
- Point gradients, an image with anisotropic filtering turned off, 0.5.0 effects, or "Across UV seams" turned off ([GUIDE_FILL.md](GUIDE_FILL.md))
- A bake priority changed from the default ([GUIDE_3D.md](GUIDE_3D.md))

A document that uses none of them keeps the earlier version. To go back to the Unity version up to 0.4.x, remove the feature and save again. Which version reads how much is in [UNITY.md](UNITY.md) and [YLP_FORMAT.md](../YLP_FORMAT.md) (in Japanese).
