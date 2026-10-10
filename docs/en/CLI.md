# Command line (yolupainter-cli)

[日本語](../CLI.md)

`yolupainter-cli` is a small console program that operates YoluPainter `.ylp` projects from commands, either without showing a window or against the app that is running.
Scripts and AI assistants can use it to read and change layers, masks and effects, get preview images, export and save. AI assistants connect to the running app directly over MCP
([the MCP document](MCP.md)); for clients that only speak MCP over standard input and output, the same program relays to the app (`yolupainter-cli mcp`).

With the installer it is placed next to the app (by default `%LOCALAPPDATA%\Programs\YoluPainter\yolupainter-cli.exe`). In the zip and tar.gz it is next to the app too. In the experimental Mac zip it is `Contents/MacOS/yolupainter-cli` inside `YoluPainter.app` ([“Mac (experimental)” in Download and updates](INSTALL.md#mac-experimental)).
The installer does not change PATH, so from Command Prompt or PowerShell go to that folder or call it by its full path.
It has the same version as the app and is replaced together with it on updates.

## Usage

```
yolupainter-cli <command> [--name value ...] [--file project.ylp [--save]] [--pretty]
yolupainter-cli batch [file|-] --file project.ylp [--save]
yolupainter-cli run-action <action.json> [--file project.ylp [--save]]
yolupainter-cli commands            list every command
yolupainter-cli schema [command]    JSON Schema of the commands (--tools: MCP tool definitions)
yolupainter-cli mcp [--port number] relay MCP on stdio to the running app
```

### Choosing the target

- `--file project.ylp`: opens that `.ylp` without the app and applies one command. The file is unchanged unless you save. With `--save` the `.ylp` is saved in place after the command succeeded
  (the previous version stays in the `<file name>-backups~` folder next to it).
- Without `--file` the target is the running YoluPainter. Turn on "Accept external commands" in the app's settings first (it is off by default).
  The command runs inside the app, and each command is one undo step of the app. If the app is not accepting commands you get an error that says how to fix it (exit code 3).
  - The app is reached over HTTP that stays inside this PC (`http://127.0.0.1:17347/mcp`). If you changed the port in the app's settings, pass the same one with `--port number`.
  - The reply is awaited for 60 seconds (change it with `--timeout seconds`). When that runs out it is unknown whether the command ran, so check with `doc.info` or `history.info` before asking again.

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
Inside a batch, layers and effects made by earlier commands can be named with `$created:<n>` ([relative references](#relative-references)).

### Applying an action (run-action)

Actions recorded in the app's Actions panel are kept as `actions/<name>.json` in the settings folder (on Windows
`%APPDATA%\YoluPainter\actions`). Such a file can be applied to a `.ylp` without the app, or to the running app (without `--file`).

```
yolupainter-cli run-action "Action 1.json" --file work.ylp --save
```

```json
{
  "format": 1,
  "name": "Action 1",
  "commands": [
    {"command": "layer.add", "args": {"kind": "fill", "name": "Wash", "fill": {"Color": "#336699ff"}, "above": "$selected"}},
    {"command": "effect.add", "args": {"layer": "$created:1", "kind": "blur", "values": {"radius": 4.0}}}
  ]
}
```

- The commands are applied to one texture set as **one undo step**. If a command is refused, everything applied before it is rolled back, the run stops and nothing is saved
  (the error's `data.index`, `data.command` and `data.completed` are as in batch).
- Only commands that change layers, masks and effects can be in an action (`layer.add`, `layer.delete`, `layer.move`, `layer.set`, `mask.add`, `mask.delete`, `mask.set`,
  `effect.add`, `effect.set`, `effect.delete`). An action holding a read, preview, export, save or undo command is refused before anything is applied. Leave `set` out, or give the same one everywhere.
- Limits: 1,000 commands per action, 4 MiB per file, names up to 100 characters. A file whose `format` is not 1 is not read.
- Exports and saves cannot be in an action and are not recorded by the app (an action edits layers, masks and effects only).
- The reply is `{"action": "<name>", "reply": "action", "set": "<set id>", "steps": [{"layer": "..."}, ...], "undo_count": 1, "can_undo": true, "saved": {...}}`.
  `steps` has one entry per command: the layer (`layer`) and effect (`effect`) it added or changed, and whether it changed nothing (`unchanged`).
- To the running app the action is sent as one `action.run` command, and one undo in the app takes all of it back. There `$selected` is the layer selected in the current texture set.
  A `.ylp` does not store the selected layer, so an action using `$selected` is refused without the app.
- The same run is available as the command `action.run` (`{"commands": [...]}`; the MCP tool `action_run`).

### Relative references

So that a recorded action works on another document too, layer and effect fields (`layer`, `above`, `parent`, `effect`) accept these besides ids and names.

| Written as | Refers to | Where |
|---|---|---|
| `$selected` | The selected layer (layer fields only) | The running app (the layer selected in the current texture set). In an action or a batch, the one selected when it started. Refused for a `.ylp` without the app |
| `$created:<n>` | The n-th (from 1) layer or effect made in the same run (one count over `layer.add` and `effect.add`) | Inside an action or a batch only. Refused for a single command |

- An effect in a layer field, or a layer in an effect field, is refused. So is a number past what was made (`data.created` gives the count).
- Other text starting with `$` is looked up as a name (text starting with `$created:` whose number cannot be read is refused).

### Replies and exit codes

The reply is JSON on standard output (`--pretty` formats it). The PNG of a preview (`preview`) is included as base64; with `--out preview.png` the PNG is written to that file and the JSON gives `png_file`.

On failure, standard output has `{"error": {"code": "...", "message": {"ja": "...", "en": "..."}, "data": {...}}}` and standard error has one line.
The language of that line is chosen with `--lang ja|en` (or the environment variables `YOLUPAINTER_LANG`, `LANG`); when it cannot be decided both languages are printed.

| Exit code | Meaning |
|---:|---|
| 0 | Success |
| 1 | The command refused (not found, value out of range, read-only set, file failure and so on; see `error.code`) |
| 2 | Bad arguments (unknown command or field, wrong type, unreadable JSON) |
| 3 | The running app cannot be reached (not running, the setting is off, or a different port) |
| 4 | A destructive command lacks confirmation (`--confirm`) |

The list of error `code`s is in [the command reference](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-ops/README.md).

## Commands

"Read" changes nothing. "Edit" changes the document and is one undo step. "Destructive" needs `--confirm`. "Replace" needs `--confirm` only when it replaces an existing file.
`set` is a texture set id or name (omit it for the current set). Layers are given by their 32-digit hex id or by name (when a name matches several layers the command refuses and lists the ids).
[Relative references](#relative-references) (`$selected`, `$created:<n>`) work too.

| Command | Arguments | Kind |
|---|---|---|
| `doc.info` | | Read |
| `doc.open` | `path`, `confirm` | Replace (when it discards unsaved changes of the open document) |
| `set.info` | `set` | Read |
| `layer.get` | `layer` | Read |
| `layer.add` | `kind` (paint, fill, group, adjustment, text), `name`, `above`, `fill`, `adjustment`, `channels`, `text` | Edit |
| `layer.delete` | `layer`, `confirm` | Destructive |
| `layer.move` | `layer`, `parent`, `to_root`, `index` | Edit |
| `layer.set` | `layer`, `name`, `visible`, `opacity`, `blend_mode`, `clipping`, `locks`, `channels`, `fill`, `adjustment`, `points` (fill layer point gradients: channel -> `space`, `spread`, `points`; `null` removes), `text` | Edit |
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
| `action.run` | `commands` (the command list of an [action](#applying-an-action-run-action)) | Edit (one undo step in all; destructive commands inside each need `confirm`) |

Field types, ranges and descriptions are printed by `yolupainter-cli schema <command>`. The effect kinds and the ranges of their values are returned by `effect.list_kinds`.
Painting operations (strokes, fills, selections) are not commands yet.

## Safety

- Destructive operations (deleting, saving over a file, replacing an existing file) are refused without changing anything unless `--confirm` is given. `--save` counts as asking for the overwrite.
- Saving is committed by one replacement from a verified temporary file. If the file was changed outside after it was opened it is not overwritten (`conflict`), and the previous version stays in the backups folder next to it.
- There is no command that runs arbitrary code. The files touched are the `.ylp` of `--file` and the paths named by the export and save commands.
- Commands to the app are accepted only from this PC (`127.0.0.1`). There is no password, so while the setting is on, programs of other accounts on this PC can connect too (see "Safety" in [the MCP document](MCP.md)).
- Without the app, Generators that read baked mesh maps or the model have no effect (their settings are kept, but they are not in previews, exports or the composite PNGs of a saved file; the reply's `notes` and `inactive_effects` say so).
  A set that uses features only the standalone application has (noise, grunge, gradient map and so on) is saved in a newer version that the Unity version up to 0.4.x cannot open ("Ranges by reader" in [the .ylp format](../YLP_FORMAT.md), Japanese); the `notes` of the save reply tell which set.
