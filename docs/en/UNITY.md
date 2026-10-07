# Working with Unity

[日本語](../UNITY.md)

## Live Link

Open a scene model (such as an avatar) from the Unity Editor on the same PC in the standalone application. A Unity 2022.3 project needs **a Live Link-enabled version of
YoluPainter for Unity (VPM package `net.yozolab.yolupainter`)**. The two exchange JSON files in a folder on the same PC; no native library is used (specification:
[LIVELINK.md](LIVELINK.md)).

1. In Unity's `YozoLab → YoluPainter → Live Link` window, choose a scene object and press Open in YoluPainter. If the standalone application is not running, it is
   started. The standalone application reads the object's FBX and texture files itself and creates one texture set per material (the original texture becomes the bottom layer
   "Original"). lilToon materials are drawn in the 3D view with Unity's values (only materials with lilToon's version value or shaders from the lilToon package; shaders that only
   look similar by name are not drawn as lilToon).
2. After changing the pose, BlendShapes or material values in Unity, press the same button to resend (the FBX is not read again; only the pose and values are applied, overwriting
   bones moved by hand in the Pose panel).
3. When you export with File → Export, the export folder starts in the folder Unity specified. Unity imports the PNG files written and asks in its window whether to assign them to
   the materials.
4. Save your work as `.ylp`. The model opened over Live Link (the FBX files, renderer and material bindings, and pose) is kept in the `.ylp` and reopens without Unity.

The standalone application opens models from Unity while "Accept Live Link from Unity" in Edit → Settings… (Ctrl+,) is on (the default; starting with `--livelink` always
accepts). The icon at the right end of the menu bar shows the state (not accepting, accepting, a document opened from Unity, something did not fit, folder unavailable); press it to
see the state as "Live Link: Accepting" and the open object as "Unity: name", and to switch between Accept and Don't accept.

- Only renderers with meshes imported from FBX files can be opened (meshes such as .asset files are not sent, with a reason in the Unity window). FBX files imported with Bake Axis
  Conversion cannot be opened.
- Opening a different object with unsaved changes asks the same question as Open (if you cancel, the reason is sent back to Unity). Anything sent from Unity while drawing or
  saving is applied afterwards.
- The exchange folder (`%LOCALAPPDATA%\YoluPainter\LiveLink` on Windows, `~/Library/Application Support/YoluPainter/LiveLink` on Mac, `~/.local/share/YoluPainter/LiveLink`
  on Linux) is accessible only to the user; folders others can enter are not used. Other programs running as the same user can place files there.
- The original texture and material assets in the Unity project are written only when you choose to assign the exported textures to the materials.

## Exchanging .ylp files with the Unity version

The `.ylp` format specification (entries, versions, and which readers accept what) is in [YLP_FORMAT.md](../YLP_FORMAT.md) (Japanese).

| Content | Handling in the standalone application |
|---|---|
| `.ylp` formats 1–8, internal document formats 1–26 | Readable. Saving updates the file to `.ylp` format 7 (format 8 only for documents that use remembered selections); document format depends on the features used (21 to 24 as described in the user channel, Noise/Grunge, and color adjustment rows, or 25 with gradient map blending; documents over 512 MiB use 26) |
| Remembered selections (per-set `selections.json` and `selection-<content id>.bin`, up to 32 per set; separate from the current selection `selection.bin`) | Editable and saveable. Only documents that use them are saved as `.ylp` format 8 (the outer `YOLUPAINTER-YLP-3` and the document format do not change); deleting them all and saving returns the file to format 7. Readers that only know format 7 (the 0.3.x standalone, and the Unity window and importer of version 0.2.0 or later) refuse it as a newer format, naming the writing app, and lose nothing. Save for Distribution removes them by default, giving a format 7 copy the Unity version can open |
| Model pose (root `pose.json`: differences from rest, held by bone name path and BlendShape names) | Editable and saveable (a state entry; the format does not change). It comes back when the same model is opened, skipping items that do not fit, with reasons. The 0.3.x standalone keeps it as an unknown entry byte for byte; the Unity version lists it as unknown and drops it when saving (only the pose is lost). The pose of a Live Link target is kept in `livelink.json` |
| Live Link target (root `livelink.json`: FBX files, renderer and material bindings, pose, target) | Saved (a state entry; the format does not change). Opening reopens the same model and pose without Unity. The 0.4.x standalone lists it as unknown and keeps it byte for byte (the model is not opened, but the texture sets are). The Unity version lists it and drops it when saving (only the target record and pose are lost) |
| Raster layers (six standard channels), groups (pass-through/isolated), masks, fills, adjustments (Invert, Levels, Hue/Saturation/Lightness), clipping, layer locks (alpha, pixels, position, all), per-channel enable and blending, Normal settings | Editable and saveable. Document/layer IDs and RGB values of transparent pixels are preserved. Locks use the document format 12 attribute layout; documents without locks retain the same bytes as before |
| User channels | Editable and saveable. Only sets with user channels are saved in document format 22. Unity versions supporting only document formats up to 21 (such as 0.2.0) cannot open that `.ylp` |
| Noise/Grunge (generator types exclusive to the Rust version, including disabled stages) | Editable and saveable. Only sets containing these stages use document format 23. Unity versions supporting only document formats up to 21 (such as 0.2.0) cannot open that `.ylp`. Removing the stages and saving returns the set to format 21 (22 with user channels). They cannot be written to `.ylsmart` (format 1) |
| Color adjustments (Gradient Map, Tone Curve, Color Balance, Brightness/Contrast, Threshold, Posterize; adjustment layer and filter stage types exclusive to the Rust version, including disabled stages) | Editable and saveable. Only sets containing these layers or stages use document format 24. Unity versions supporting only document formats up to 21 (such as 0.2.0) cannot open that `.ylp`. Removing them and saving again returns the set to 21 (22 with user channels, 23 with Noise/Grunge). They cannot be written to `.ylsmart` (format 1) |
| Mesh maps (per-set `meshmap-<kind>.bin`) | Loaded and saved. Only newly baked, unsaved maps are written; maps present when opening are preserved as-is. Maps whose model or settings differ from the current ones are marked stale and not used |
| Filters (layers/masks), Anchor, 2D/3D paths | Editable and saveable. Settings are preserved and evaluated during compositing, so effects appear on screen and in exported composite PNGs |
| Generators using mesh maps or images; fill images, projections, gradients; decals | Editable and saveable. Evaluation receives mesh maps baked for the current conditions, the model root, and image assets. On opening, sets with effects lacking inputs (missing, stale, or unverifiable maps, or images absent from the project's assets) are read-only: the saved composite is displayed, the original document is preserved, and missing inputs are listed. Once inputs are available (by baking, loading a model, or adding image assets), the same set becomes editable. Settings that can be fixed within the document, such as no selected anchor or no ID colors, do not make a set read-only |
| Manual ID colors | Editable and saved with the project. They are written in document format 21, which the Unity version reads, so a document that only adds manual ID colors still opens in the Unity version |
| Appearance of read-only sets | Displays the saved Color composite. If it is missing or unreadable, an empty view and a reason are shown |
| Assets (`resources.json` and `resources/`: images, brushes, materials, Smart Materials, Smart Masks) | Loaded and saved. Rewritten only when the project's assets change, with YoluPainter as the writer name (standalone in the Unity version field); otherwise the original bytes are preserved. Smart Materials and Smart Masks added by this application use `.ylsmart` (format 1), readable by Unity |
| Unedited sets, embedded resources, unknown additional entries | Original data is preserved when saving |
| Sets whose groups are nested deeper than 64 levels (groups stacked on one chain) | Opening is refused with a reason, and the file is not touched. The Unity version sets no nesting limit, so a `.ylp` nested deeper than 64 levels in Unity cannot be opened here. Editing and PSD import also stop at 64 levels |
| Newer formats, unknown values, damaged files | Opening is refused with a reason |

To return a file to Unity, use a version that reads `.ylp` format 7 and document format 21. If a `.ylp` contains sets with user channels, Noise/Grunge, or color adjustments, remove those features and save again first. If it uses remembered selections, delete them all and save again, or use Save for Distribution with Remembered selections removed. There is no conversion to formats for older Unity versions. Read-only sets retain their original editing data when saved instead of being replaced by flattened composites. However, saved images may not reflect a fresh evaluation of the latest effects.

Edited sets are saved in document format 21 (22 with user channels, 23 with Noise/Grunge, 24 with color adjustments, 25 with gradient map blending), along with composites of the standard channels in use. Undo history is not saved. Identical ZIP bytes for the entire `.ylp`, window layouts, and model connection state across the two applications are not guaranteed.

The version immediately before an overwrite is kept in `<filename>-backups~/` beside the file. By default all backups are retained without automatic deletion. Backups to Keep in Edit → Settings… lets you retain the newest 0–1000 backups (0 keeps none); only older backups beyond that count are removed. The destination filename must end in `.ylp`; missing destination folders are created. If another application has changed the file since it was opened, overwriting is refused. Do not edit the same file simultaneously in both applications: save, then reopen it in the other application.
