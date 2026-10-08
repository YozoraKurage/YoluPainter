---
name: yolupainter
description: Work on the project open in the YoluPainter texture painting app through its MCP tools - read texture sets and layers, add and edit layers, masks, filters and generators, look at channel previews, export textures and save. Use when the user asks to inspect or change their YoluPainter project.
---

# YoluPainter

The `yolupainter` tools talk to the YoluPainter app running on this PC (`http://127.0.0.1:17347/mcp`). They act on the project the app has open, and the user sees every change in the app.

## Before the first call

- If a tool fails with code `io` and "Cannot reach a running YoluPainter", ask the user to start the app and turn on Settings > Accept external commands. If they changed the port there, the connection in the assistant's settings must use the same port.

## Reading

- `doc_info`: the file, the texture sets, the current set and whether there are unsaved changes.
- `set_info`: size, channels and the layer stack from top to bottom. Refer to a layer by its id when two layers share a name (`ambiguous` lists the candidates).
- `layer_get`, `effect_get`, `history_info`.
- `preview`: a PNG of a channel (`max_edge` keeps it small). Look at it after visible edits.

## Editing

- `layer_add`, `layer_set`, `layer_move`, `mask_add`, `mask_set`, `effect_add`, `effect_set`.
- Each editing tool is one step of the app's undo history; `undo` and `redo` revert and repeat them.
- `action_run` applies several of these at once (`commands`: a list of `{"command": "layer.add", "args": {...}}`) as one undo step; if one is refused, all of them are rolled back. Inside the list `$created:1` names the first layer or effect the list made.
- Before `effect_add` or `effect_set`, read `yolupainter://ops/effect-kinds` (or call `effect_list_kinds`) for the kinds, their parameters and value ranges.

## Destructive tools need consent

- `layer_delete`, `mask_delete`, `effect_delete`, `save`, and `save_as`, `export_channels`, `export_textures`, `export_psd` when they replace a file need `confirm: true`.
- A destructive command inside `action_run` needs its own `confirm: true`.
- Ask the user before passing `confirm: true`. Without it the tool changes nothing and answers `confirm_required`.
- `save` keeps the previous version in the backups folder next to the file.
- Use absolute paths for `save_as` and the export tools.

## When a tool is refused

- `busy`: the user is drawing, a save is running, or the app has all its connections in use. Wait a moment and call again.
- `read_only`: the texture set cannot be edited; the message says why.
- `unsupported` from `doc_open`: the app does not switch documents; ask the user to open the file in the app.

## Not available

Drawing strokes, fills, selections, baking and opening another document. Point the user to the app for those.

## More

- `yolupainter://docs/mcp`: these tools and the connection.
- `yolupainter://docs/guide`: the user guide's entry page, listing its pages. Each page is its own resource:
  `yolupainter://docs/guide-layers` (layers, masks, channels), `yolupainter://docs/guide-fill` (fill layers, filters and generators, assets, actions),
  `yolupainter://docs/guide-files` (saving, exporting, PSD), `yolupainter://docs/guide-3d` (3D view, models, baking),
  `yolupainter://docs/guide-paths`, `yolupainter://docs/guide-settings`, `yolupainter://docs/guide-start`, `yolupainter://docs/guide-paint`,
  `yolupainter://docs/guide-select`, `yolupainter://docs/guide-keys`.
