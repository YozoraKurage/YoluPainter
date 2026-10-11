# Working with Unity

[日本語](../UNITY.md)

## Live Link

Open a scene model (such as an avatar) from the Unity Editor on the same PC in the standalone application. A Unity 2022.3 or later project needs **the Unity Bridge (VPM package
`net.yozolab.yolupainter`)**. The bridge is released with the same version number as the standalone application, so update both to the same version. The two exchange JSON files in
a folder on the same PC; no native library is used (specification: [LIVELINK.md](LIVELINK.md)).

1. In Unity, right-click an object in the Hierarchy and choose Open in YoluPainter (also under the GameObject menu), or choose an object in the `YozoLab → YoluPainter → Live Link` window and press
   Open in YoluPainter. If the standalone application is not running, it is started. The standalone application reads the object's FBX and texture files itself and creates one
   texture set per material (the original texture becomes the bottom layer "Original"; a PSD keeps its layers). lilToon materials are drawn in the 3D view with Unity's values (only
   materials with lilToon's version value or shaders from the lilToon package; shaders that only look similar by name are not drawn as lilToon).
2. After changing the pose, BlendShapes or material values in Unity, press the same button to resend (the FBX is not read again; only the pose and values are applied, overwriting
   bones moved by hand in the Pose panel or in Pose mode. If an original texture file changed, untouched sets get it again; touched sets are left as they are, with a notice).
3. When you choose the "lilToon" Output Template in the File → Export Textures… window and export, the Output Path starts at the folder Unity specified. Of the PNG files written, Unity imports those that
   belong to a lilToon material property (`_MainTex`, `_BumpMap`, and so on) and asks in its window whether to assign them to the materials. The other templates and the per-channel PNG
   files are not sent back to Unity.
4. Save your work as `.ylp`. The model opened over Live Link (the FBX files, renderer and material bindings, and pose) is kept in the `.ylp` and reopens without Unity.
   Whether the material values received from Unity are kept in the `.ylp` is set by "Save material values received from Unity" in the same section of the settings (on by default).
   Save for Distribution with "Model reference" removed also leaves this record out of the copy ([SAVE_FOR_DISTRIBUTION.md](../SAVE_FOR_DISTRIBUTION.md), Japanese).

The standalone application opens models from Unity while "Accept Live Link from Unity" under Live Link & Commands in Edit → Settings… (Ctrl+,) is on (the default; starting with `--livelink` always
accepts). The icon at the right end of the menu bar shows the state (not accepting, accepting, a document opened from Unity, something did not fit, folder unavailable); press it to
see the state as "Live Link: Accepting" and the open object as "Unity: name", and to switch between Accept and Don't accept. The Live Link item in the File menu switches it too.

- Only renderers with meshes imported from FBX files can be opened (meshes such as .asset files are not sent, with a reason in the Unity window). FBX files imported with Bake Axis
  Conversion cannot be opened.
- Opening a different object with unsaved changes asks the same question as Open (if you cancel, the reason is sent back to Unity). Anything sent from Unity while drawing or
  saving is applied afterwards.
- The exchange folder (`%LOCALAPPDATA%\YoluPainter\LiveLink` on Windows, `~/Library/Application Support/YoluPainter/LiveLink` on Mac, `~/.local/share/YoluPainter/LiveLink`
  on Linux) is accessible only to the user; folders others can enter are not used. Other programs running as the same user can place files there.
- The original texture and material assets in the Unity project are written only when you choose to assign the exported textures to the materials.

## Exchanging .ylp files with the Unity version

From 0.5.0, the Unity Bridge does not open `.ylp` files. Painting inside Unity, importing `.ylp` and `.ylsmart` files, reading and writing PSD files, and baking mesh maps were removed;
the package now only does Live Link and applying textures to materials. Painting, opening and saving `.ylp` files is done in the standalone application. A `.ylp` placed in Assets is an
ordinary file to Unity.

What the Unity version up to 0.4.x can read when it opens a `.ylp` (the outer version, the format of the contents, the document version, and how unknown entries are handled) is in the
"Ranges by reader" table in [YLP_FORMAT.md](../YLP_FORMAT.md) (Japanese). That version reads document versions up to 21, so a `.ylp` that contains a set using user channels, Noise/Grunge,
color adjustments, the 0.5.0 effects, and so on is refused without touching the file. To go back to it, remove those features in the standalone application and save again (the version
returns to 21). A `.ylp` that uses remembered selections (format 8) is refused too, so delete them all and save again, or use Save for Distribution with Remembered selections removed
([SAVE_FOR_DISTRIBUTION.md](../SAVE_FOR_DISTRIBUTION.md), Japanese). There is no conversion to formats for older Unity versions.
