# Live Link (file exchange)

[日本語](../LIVELINK.md)

This is the specification of how the Unity Bridge in the Unity Editor (VPM package `net.yozolab.yolupainter`) and the standalone YoluPainter exchange JSON files in a folder on the same PC.
Unity writes a **request** with the chosen target's (a scene object's) **FBX paths, bone values, material values and texture paths**, and the standalone application picks it up and opens it.
When you export in the standalone application, it writes a **reply** listing the PNG files it wrote, and Unity picks it up and imports them. Meshes and pixels are not sent; each side reads the files itself.

Usage is described in [UNITY.md](UNITY.md#live-link). This document is for people who build either side or inspect the exchanged files.

## Folder

Both sides work out the same path on their own.

| OS | Folder |
|---|---|
| Windows | `%LOCALAPPDATA%\YoluPainter\LiveLink\` |
| macOS | `~/Library/Application Support/YoluPainter/LiveLink/` |
| Linux and others | `${XDG_DATA_HOME:-~/.local/share}/YoluPainter/LiveLink/` (the default if `XDG_DATA_HOME` is not absolute) |

- If the environment variable `YOLUPAINTER_LIVELINK_DIR` (an absolute path) is set, both sides use that folder instead (tests, or two applications side by side).
- Inside are `inbox/` (Unity → standalone), `claimed/` (requests the standalone application picked up, and the marks `<request file name>.lock` that show a request is being picked up; Unity neither reads nor writes them), `outbox/` (standalone → Unity), and the presence file `presence.json`.
- The folder is accessible only to the user: on Unix, mode 0700 and owned by the user (Unity runs `chmod 0700` when it creates it); on Windows, a DACL allowing only that user.
  The standalone application does not use a folder others can enter or that someone else owns (the reason appears on the entrance icon).
- Files are always written as `<name>.tmp`, closed, and then renamed to the final name. Readers ignore names ending in `.tmp` (they never read a half-written file).

## Presence `presence.json`

While the standalone application accepts requests, it rewrites this file every 2 seconds, and removes it when it stops accepting or exits.

```json
{ "format": 1, "app": "YoluPainter", "version": "0.6.0", "pid": 1234, "updated": "2026-10-07T12:00:00Z" }
```

Unity treats the standalone application as running if `updated` (UTC) is less than 6 seconds old. Otherwise it starts the standalone application with `--livelink` and then places
the request (it need not wait; the standalone application picks it up once it is running).

## Request `inbox/<id>.json` (Unity → standalone)

```json
{
  "format": 1,
  "kind": "open",
  "id": "8f0c2a4e-0000-4000-8000-000000000001",
  "bridge": { "version": "0.6.0", "unity": "2022.3.22f1" },
  "project": { "root": "C:/Work/MyProject", "name": "MyProject" },
  "target": {
    "key": "GlobalObjectId_V1-2-…",
    "name": "Avatar",
    "export_dir": "C:/Work/MyProject/Assets/YoluPainter/Avatar"
  },
  "root": { "world": [1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1] },
  "models": [
    { "id": 0, "fbx": "C:/Work/MyProject/Assets/Avatar/Body.fbx", "guid": "0123456789abcdef0123456789abcdef",
      "import": { "global_scale": 1.0, "use_file_scale": true, "bake_axis_conversion": false,
                  "import_blend_shapes": true, "preserve_hierarchy": false } }
  ],
  "renderers": [
    { "path": "Body", "model": 0, "node": "Body", "enabled": true, "skinned": true,
      "blend_shapes": { "eye_close": 0.0, "smile": 35.0 }, "materials": [0, 1] }
  ],
  "bones": [
    { "model": 0, "node": "Armature/Hips", "local": { "t": [0.0, 0.9, 0.0], "r": [0.0, 0.0, 0.0, 1.0], "s": [1.0, 1.0, 1.0] } }
  ],
  "materials": [
    { "key": "guid:0123456789abcdef0123456789abcdef/fileid:2100000", "name": "Body",
      "shader": { "name": "Hidden/lilToonCutout", "guid": "…", "package": "jp.lilxyzw.liltoon", "version": "2.3.4", "keywords": [], "render_queue": 2450 },
      "values": { "floats": { "_Cutoff": 0.5 }, "colors": { "_Color": [1, 1, 1, 1] }, "vectors": {}, "ints": { "_lilToonVersion": 45 } },
      "textures": [
        { "property": "_MainTex", "path": "C:/Work/MyProject/Assets/Avatar/body.png", "guid": "…",
          "srgb": true, "normal_map": false, "scale": [1, 1], "offset": [0, 0] },
        { "property": "_MatCapTex", "path": null }
      ] }
  ],
  "refused": [ { "path": "Accessory", "reason": "mesh_not_from_fbx" } ]
}
```

- UTF-8 JSON. All paths are absolute and separated by `/`. Numbers are JSON numbers (never NaN or infinity; values too large for f32 are refused as not finite).
- `format` is 1 and `kind` is `"open"` (open; a resend if a document of the same `target.key` is open). Unknown keys are skipped.
- `id`: a new ID per request (1–64 characters of letters, digits, `-` and `_`), used to name replies.
- `target.key`: the identity of the target scene object (`GlobalObjectId`), which tells resends of the same target apart. `name` is for display; `export_dir` is the default export folder.
- `root.world` (optional): the 4×4 matrix (column-major) from Unity's world to the target root (the target root's `worldToLocalMatrix`).
- `models[]`: the FBX files the target uses. `id` is what `renderers[].model` and `bones[].model` point to. If the same FBX is used by two groups, list it twice (the standalone
  application reads it once and places its bones and meshes twice). `import` holds the ModelImporter settings (missing fields take Unity's defaults).
- `renderers[]`: renderers. `path` is the path from the target root (for display and reasons), `node` is the path of the mesh node inside the FBX (see "Paths"), `enabled` is the
  visibility, `blend_shapes` maps BlendShape names to weights (0–100, as in Unity), and `materials` lists `materials[]` indices in submesh order.
- `bones[]`: bone values. `node` is the path of the bone node inside the FBX (empty means the node at the FBX root), and `local` is the local transform relative to the parent bone
  in the FBX (`t`, `r` (x, y, z, w), `s`). Bones not listed keep the values from the FBX. When the Transform at the FBX root is the target itself (or an ancestor of the
  target), Unity does not send the root node (empty `node`).
- `materials[]`: Unity materials. `key` identifies the material (the key texture sets are bound to; see the table below), and `shader` gives the shader name, asset GUID,
  `package` (the name of the UPM package containing the shader asset; empty or missing for shaders in `Assets` and built-in shaders), version, keywords and render queue. `values` holds property values (`floats`, `colors`, `vectors`, `ints`) and `textures` gives the texture file per texture
  property (`srgb` and `normal_map` come from the TextureImporter; `scale` and `offset` are tiling and offset). Textures that exist only inside Unity (generated without a file,
  RenderTextures) have `path` set to `null`.
- `refused[]`: renderers Unity could not send (reasons are listed below).

| `materials[].key` | Material |
|---|---|
| `guid:<32 hex digits>/fileid:<number>` | A material asset |
| `object:<GlobalObjectId>` | A material inside the scene (not an asset) |
| `instance:<InstanceID>` | A material whose GlobalObjectId is empty (an identity valid only in that Unity session) |
| `none` | Submeshes without a material (the standalone application makes them the no-material group) |

On a resend or when a `.ylp` is reopened, the standalone application binds sets again by the identifiers for asset keys and by the material name for other keys.
- Limits (exceeding them is refused as `too_large`): 16 MiB per file; 1,024 renderers, models and refused renderers; 16,384 bones; 1,024 materials; 256 textures per material;
  4,096 BlendShapes per renderer and values per kind per material; 8 KiB per string.

### Paths (nodes inside the FBX)

- A path joins names with `/` from the root of the model as imported by Unity.
- When the import setting `preserveHierarchy` is off (the default) and the FBX root has exactly one child, Unity makes that child the prefab root (its name disappears from
  paths and its transform moves to the prefab root). The standalone application reads `import.preserve_hierarchy` and follows paths from the same root.
- A path that cannot be resolved to one node because of siblings with the same name is `ambiguous_bone`; a path that is not found is `bone_not_found`.
- On import, Unity renames siblings with the same name to `Twin`, `Twin 1`, and so on. A path segment of the form `<name> <number>` that is not in the FBX, where the FBX has two or
  more siblings named `<name>`, is also `ambiguous_bone` (which sibling was renamed cannot be determined from the FBX).

### How Unity determines bone values

- Only renderers whose mesh asset path (`AssetDatabase.GetAssetPath(mesh)`) ends in `.fbx` are sent; others go to `refused` as `mesh_not_from_fbx`.
- The path of a bone inside the FBX comes from the Transform in the FBX model asset obtained with `PrefabUtility.GetCorrespondingObjectFromOriginalSource(transform)`, as the
  name path from its root. If that is not available (for example, an unpacked prefab), the bone is searched by name path from the Transform at the FBX root; if it cannot be
  resolved to one, the renderer is refused (`bone_not_found`, or `ambiguous_bone` when siblings share a name).
- `local` is the bone Transform relative to the Unity Transform of its parent node in the FBX (`parent.worldToLocalMatrix * bone.localToWorldMatrix` as T, R, S), even if the
  hierarchy has been re-parented in Unity. The node at the FBX root is relative to the target root.
- Coordinates are Unity's (left-handed, Y up, meters).

### Import settings

- The standalone application reads FBX files with Unity's orientation and size: Unity units = meters × `global_scale` × (1 if `use_file_scale`, otherwise 1 ÷ the file's unit in
  meters; 100 for an FBX whose UnitScaleFactor is 1, i.e. centimeters).
- An FBX with `bake_axis_conversion` set cannot be matched; its renderers are reported as `unsupported_import` and left out.
- An FBX with `import_blend_shapes` off is read without BlendShapes.
- Submesh order is the same as Unity's (the order of the node's material slots, skipping slots without faces).

## How the standalone application accepts requests

- It accepts requests while the setting "Accept Live Link from Unity" is on (the default) or when started with `--livelink`, and checks `inbox/*.json` every 0.5 seconds.
- To pick a request up, it first creates `claimed/<request file name>.lock` (the mark that the request is being picked up) in a way that always fails if the name already exists
  (exclusive creation). Only the application that created the mark moves the request from `inbox/` to `claimed/` and takes it, so even if two standalone applications run, a single
  request is taken only by the one that created the mark (whether a rename succeeded does not decide it). When done, it removes the request from `claimed/` and then removes the mark.
- Requests and marks left in `claimed/` for more than a day (left by a crashed process) are cleaned up when accepting starts. If the request stayed in `inbox/` and only its
  mark was left for more than a day (a crash right after creating the mark), the next application can pick it up after that cleanup.
- Requests that arrive while drawing, while saving, or while another model is loading are not refused; they stay in `claimed/` and are applied afterwards.
- If a document of the same `target.key` is open, the request is a **resend**: the pose, BlendShapes, material values and visibility are applied. An FBX is not read again while its
  path and `guid` stay the same (if visibility, the meshes used, or material assignments change, the model is rebuilt from the FBX already read). A resent pose also overwrites
  bones moved by hand (as one undo step).
- For a different target with unsaved changes, it asks the same question as Open. If you cancel opening it replies `refused` (`declined`); if you discard the changes it opens a new
  project.
- If the user replaces the document (New, Open) while a request is loading, the result is not put into the new document and the reply is `refused` (`declined`). Turning off
  acceptance also cancels the requests that are not applied yet and the ones that are loading, with `declined` (reopening a `.ylp` continues).
- Several FBX files become one model (with two or more, the root bone names are prefixed with the FBX number). Bones can be moved in the Pose panel.
- One texture set per material `key` (renderers using the same Unity material share a set).
- Original texture: the file of the Color destination (`_MainTex`) is read (PNG, TGA, JPG, PSD) and added as the bottom layer "Original" of newly created sets
  and of the untouched first set. Materials without a texture, or with a texture that exists only inside Unity, start white; unreadable files start white with a reason.
- When the original texture is a PSD (sRGB), the set gets the PSD's layers instead of a flattened "Original". It is read as the same copy as File → Import of a PSD;
  the PSD's layers come at the bottom and the set's empty layer stays above them. The set keeps the PSD's canvas size (layers are not resized). What the import drops
  or changes is listed in the notice with the same names as the import check window (no window is shown). The original PSD is only read; exporting a PSD to the
  same file asks before replacing it. A PSD that cannot be imported as layers (for example over the budget) becomes a flattened "Original", with the reason in the
  notice. The pixels of the originals read for one request (flattened pictures and the documents of PSDs kept as layers) are limited to 512 MiB in total; a PSD that would go
  over the remainder is flattened (and if even that does not fit, the set starts white with the reason). When several materials use the same PSD, each gets its own document
  and each is counted. A linear PSD (`srgb` is false) and PSDs in slots other than Color are read flattened.
- If a resend finds that the original texture file changed (path, modification time, size or `srgb`), a set that is still as it was when the original was put in
  (nothing drawn or changed) gets the new original (white if the texture is gone). If the changed file cannot be read (damaged, being written, and so on), the set is not changed (it does not turn white),
  the notice says "<set>: the original "body.psd" cannot be read, so the set is kept", and the file is read again on the next resend. A set you have touched is not changed; the notice says "<set>: the original "body.psd" has changed" (once per
  change). Whether a set is untouched is remembered only while the application is open, so sets reopened from a `.ylp` count as touched (changes after reopening
  are reported).
- lilToon material values become the set's received look (read by the 3D view's lilToon rendering; `_MainTex` shows the standalone application's Color, and textures of other slots
  are read from their files, up to 2048 on the long side and 256 MiB in total; a resend does not read a file again while its path, modification time and size stay the same).
  Materials that are not lilToon get no received look.
- Whether a material is lilToon is not decided by the shader name. Only when `values` has `_lilToonVersion` (the version value lilToon shaders have) or `shader.package` is
  `jp.lilxyzw.liltoon` is it rendered as lilToon.
- The `.ylp` keeps the applied request with the current pose and without material values (except `_lilToonVersion`) in the root entry `livelink.json`, so it reopens without Unity
  ([YLP_FORMAT.md](../YLP_FORMAT.md), Japanese).
  The material values received stay in the set's `look.json` while "Save material values received from Unity" is on (the default; textures' pixels are not stored, and they are read from their files on reopening).
  A copy made with Save for Distribution with "Model reference" removed does not contain `livelink.json` either ([SAVE_FOR_DISTRIBUTION.md](../SAVE_FOR_DISTRIBUTION.md), Japanese).

## Reply `outbox/<id>-<n>.json` (standalone → Unity)

```json
{ "format": 1, "request": "8f0c2a4e-0000-4000-8000-000000000001", "kind": "exported",
  "app": { "version": "0.6.0" },
  "problems": [ { "path": "Accessory", "reason": "bone_not_found" } ],
  "files": [ { "material": "guid:0123456789abcdef0123456789abcdef/fileid:2100000", "property": "_MainTex",
               "path": "C:/Work/MyProject/Assets/YoluPainter/Avatar/Avatar_Body_Main.png", "srgb": true, "normal_map": false } ] }
```

- `kind`: `opened` (accepted and opened, or a resend applied; anything that did not fit is in `problems`), `refused` (not accepted; reasons in `problems`), `exported` (you exported).
- `<n>` counts from 0 per request. A reply to an unreadable request uses the `inbox/` file name (before the extension) as `request`.
- `exported`: when you export a Live Link target's document, the Output Path in the Export Textures window starts at `target.export_dir` (or the folder you chose later). Of the PNG files written, images of the
  lilToon template (`Main` → `_MainTex`, `Normal` → `_BumpMap`, `Smoothness` → `_SmoothnessTex`, `Metallic` → `_MetallicGlossMap`, `Emission` → `_EmissionMap`) and slot images of
  the lilToon packing go into `files`. Images without a matching property, and images of the `none` group, are left out.

## How Unity receives replies

- After sending a request, and while the Live Link window is open, Unity checks `outbox/` every second and removes the replies it has read.
- For `exported`, Unity imports the `files` and applies `srgb` and `normal_map` to the TextureImporter (paths outside `Assets` are not imported, with a reason). Whether to assign
  them to materials is confirmed with a list of changes (material, property, old texture → new texture), and assignments can be undone with Unity's Undo.

## Reason words (`reason`)

| Word | Meaning |
|---|---|
| `mesh_not_from_fbx` | The mesh does not come from an FBX (.asset and so on) |
| `bone_not_found` | A bone or mesh node is not found in the FBX |
| `ambiguous_bone` | Siblings share a name, so the path does not resolve to one |
| `unsupported_import` | The import setting (`bake_axis_conversion`) cannot be matched |
| `fbx_unreadable` | The FBX cannot be read |
| `texture_unreadable` | A texture file cannot be read |
| `too_large` | Over a limit |
| `format_unknown` | Another format or kind, not readable as JSON, or not following the rules |
| `busy` | While drawing or saving (the current standalone application does not reply with it; it applies the request afterwards) |
| `declined` | The user cancelled opening: another target with unsaved changes that were not discarded, the document was replaced while loading, or acceptance was turned off |

Unknown words are shown as they are.

## Differences from Live Link up to 0.4

- Poses changed in Unity no longer appear immediately, and painting no longer shows in Unity's view while you draw (resend to apply; exported PNG files go back to Unity).
- Meshes that do not come from an FBX (.asset and so on) and models whose import bakes the axis conversion (`bake_axis_conversion`) cannot be opened.
- The persistent connection (native bridge library and shared memory) is no longer used.
