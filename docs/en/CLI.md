# Command line (yolupainter-cli)

[日本語](../CLI.md)

`yolupainter-cli` is a small console program that operates YoluPainter `.ylp` projects from commands, either without showing a window or against the app that is running.
Scripts and AI assistants can use it to read and change layers, masks and effects, get preview images, export and save. The MCP server for AI assistants is the same program
(`yolupainter-cli mcp`) and is described in [the MCP document](MCP.md).

With the installer it is placed next to the app (by default `%LOCALAPPDATA%\Programs\YoluPainter\yolupainter-cli.exe`). In the zip and tar.gz it is next to the app too.
The installer does not change PATH, so from Command Prompt or PowerShell go to that folder or call it by its full path.
It has the same version as the app and is replaced together with it on updates.

## Usage

```
yolupainter-cli <command> [--name value ...] [--file project.ylp [--save]] [--pretty]
yolupainter-cli batch [file|-] --file project.ylp [--save]
yolupainter-cli commands            list every command
yolupainter-cli schema [command]    JSON Schema of the commands (--tools: MCP tool definitions)
yolupainter-cli mcp                 MCP server on stdio
```

### Choosing the target

- `--file project.ylp`: opens that `.ylp` without the app and applies one command. The file is unchanged unless you save. With `--save` the `.ylp` is saved in place after the command succeeded
  (the previous version stays in the `<file name>-backups~` folder next to it).
- Without `--file` the target is the running YoluPainter. Turn on "Accept external commands" in the app's settings first (it is off by default).
  The command runs inside the app, and each command is one undo step of the app. If the app is not accepting commands you get an error that says how to fix it (exit code 3).

### Passing arguments

Commands are named like `layer.set` (the MCP tool name `layer_set` works too). Pass arguments as `--name value`; the value is read according to the command's JSON Schema
(`123` for a string field stays a string, a number field is a number).

```
yolupainter-cli layer.set --file work.ylp --layer Base --opacity 0.5 --blend-mode Multiply --save
yolupainter-cli effect.add --file work.ylp --layer Base --kind blur --values.radius 4
yolupainter-cli layer.add --file work.ylp --kind fill --name Wash --fill.Color "#336699"
yolupainter-cli layer.delete --file work.ylp --layer Tint --confirm
yolupainter-cli export.channels --file work.ylp --dir out --channels Color,Normal
```

- Boolean fields: `--confirm` (true), `--visible false`, `--no-confirm`. Array fields: repeat the option (`--channels Color --channels Normal`), separate with commas, or pass a JSON array.
- Nested fields: follow them with `.` (`--values.radius 4`) or pass JSON (`--values '{"radius":4}'`).
- To pass everything as JSON, put one of `'{"layer":"Base","opacity":0.5}'`, `@args.json` or `-` (standard input) after the command name. Options written after it override it.
- Relative paths of `--file` and of output locations (`--dir`, `--path`) start at the current folder (change it with `--cwd`). A relative path that climbs out of the working folder with `..` is refused
  (use an absolute path to write there).

### Running several commands (batch)

Apply several commands in one run without opening and closing the `.ylp` each time. The input is a JSON array, or one command per line (blank lines and lines starting with `#` are skipped).

```
yolupainter-cli batch commands.jsonl --file work.ylp --save
```

```json
{"command": "layer.add", "args": {"kind": "fill", "name": "Wash", "fill": {"Color": "#336699"}}}
{"command": "layer.set", "args": {"layer": "Wash", "opacity": 0.5}}
{"command": "preview", "args": {"max_edge": 512}}
```

If a command fails the run stops there and nothing is saved (the error's `data.index` tells which one, `data.completed` how many were done).
The reply is `{"replies": [...], "saved": {...}}` (`saved` with `--save`).

### Replies and exit codes

The reply is JSON on standard output (`--pretty` formats it). The PNG of a preview (`preview`) is included as base64; with `--out preview.png` the PNG is written to that file and the JSON gives `png_file`.

On failure, standard output has `{"error": {"code": "...", "message": {"ja": "...", "en": "..."}, "data": {...}}}` and standard error has one line.
The language of that line is chosen with `--lang ja|en` (or the environment variables `YOLUPAINTER_LANG`, `LANG`); when it cannot be decided both languages are printed.

| Exit code | Meaning |
|---:|---|
| 0 | Success |
| 1 | The command refused (not found, value out of range, read-only set, file failure and so on; see `error.code`) |
| 2 | Bad arguments (unknown command or field, wrong type, unreadable JSON) |
| 3 | The running app cannot be reached (not running, or the setting is off) |
| 4 | A destructive command lacks confirmation (`--confirm`) |

The list of error `code`s is in [the command reference](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-ops/README.md).

## Commands

"Read" changes nothing. "Edit" changes the document and is one undo step. "Destructive" needs `--confirm`. "Replace" needs `--confirm` only when it replaces an existing file.
`set` is a texture set id or name (omit it for the current set). Layers are given by their 32-digit hex id or by name (when a name matches several layers the command refuses and lists the ids).

| Command | Arguments | Kind |
|---|---|---|
| `doc.info` | | Read |
| `doc.open` | `path`, `confirm` | Replace (when it discards unsaved changes of the open document) |
| `set.info` | `set` | Read |
| `layer.get` | `layer` | Read |
| `layer.add` | `kind` (paint, fill, group, adjustment), `name`, `above`, `fill`, `adjustment`, `channels` | Edit |
| `layer.delete` | `layer`, `confirm` | Destructive |
| `layer.move` | `layer`, `parent`, `to_root`, `index` | Edit |
| `layer.set` | `layer`, `name`, `visible`, `opacity`, `blend_mode`, `clipping`, `locks`, `channels`, `fill`, `adjustment` | Edit |
| `mask.add` | `layer` | Edit |
| `mask.delete` | `layer`, `confirm` | Destructive |
| `mask.set` | `layer`, `enabled`, `inverted`, `density` | Edit |
| `effect.get` | `layer`, `effect` | Read |
| `effect.add` | `layer`, `target`, `kind`, `values`, `channels`, `strength`, `enabled`, `index` | Edit |
| `effect.set` | `layer`, `effect`, `kind`, `values`, `channels`, `strength`, `enabled`, `index` | Edit |
| `effect.delete` | `layer`, `effect`, `confirm` | Destructive |
| `effect.list_kinds` | | Read |
| `history.info` | `set` | Read |
| `undo`, `redo` | `set`, `steps` | Edit |
| `preview` | `set`, `channel`, `max_edge` | Read |
| `export.channels` | `set`, `channels`, `dir`, `name`, `confirm` | Replace |
| `export.textures` | `set`, `template`, `dir`, `name`, `confirm` | Replace |
| `export.psd` | `set`, `path`, `channel`, `mode`, `confirm` | Replace |
| `save` | `confirm` | Destructive (overwrites the open `.ylp`) |
| `save_as` | `path`, `confirm` | Replace |

Field types, ranges and descriptions are printed by `yolupainter-cli schema <command>`. The effect kinds and the ranges of their values are returned by `effect.list_kinds`.
Painting operations (strokes, fills, selections) are not commands yet.

## Safety

- Destructive operations (deleting, saving over a file, replacing an existing file) are refused without changing anything unless `--confirm` is given. `--save` counts as asking for the overwrite.
- Saving is committed by one replacement from a verified temporary file. If the file was changed outside after it was opened it is not overwritten (`conflict`), and the previous version stays in the backups folder next to it.
- There is no command that runs arbitrary code. The files touched are the `.ylp` of `--file` and the paths named by the export and save commands.
- Commands to the app are accepted only from programs of the same user on the same PC (see "Safety" in [the MCP document](MCP.md)).
- Without the app, Generators that read baked mesh maps or the model have no effect (their settings are kept, but they are not in previews, exports or the composite PNGs of a saved file; the reply's `notes` and `inactive_effects` say so).
  A set that uses Rust-only effects (noise, grunge, gradient map and so on) is saved in a newer format that the Unity package (0.2.0) cannot open; the `notes` of the save reply tell which set.
