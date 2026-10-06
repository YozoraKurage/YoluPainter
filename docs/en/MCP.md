# Operating from an AI assistant (MCP)

[日本語](../MCP.md)

`yolupainter-cli mcp` is an MCP server that lets an AI assistant (Claude Desktop, Claude Code, the ChatGPT desktop app and others) operate YoluPainter.
It is a local server spoken to over standard input and output and never goes onto the network. The assistant can read the layers, masks and effects of a `.ylp`, change values, look at preview images, export and save.
The tools are the same 25 as the [command line](CLI.md) commands (the `.` in names becomes `_`).
Painting operations (strokes, fills, selections) are not available yet.

The target is chosen per tool with the optional argument `file`:

- With `file`: the `.ylp` is opened and operated on directly, without the app. Edits stay in the server's memory until `save` (which needs `confirm: true`) is called, so the file is unchanged until then.
  Later calls with the same `file` use the same document, so `undo` works. Up to 8 files are open at once; when all of them have unsaved edits another file is refused until one is saved.
  A relative `file` starts at the folder the server was started in, and relative paths such as export folders start at the folder of that `.ylp` (the folder it was first opened from; saving under a name in another folder does not change it).
  - When the file is changed outside: without unsaved edits, the next call reopens it and reads the new content. With unsaved edits it is not reopened and `save` is refused with `conflict`.
    To discard those edits and reload, call `doc_open` with the same `.ylp` as `file` and `path`, and `confirm: true` (the undo steps are discarded too).
  - After `save_as`: that document has moved to the new file, so pass the new path as `file` from then on. The old path as `file` is a separate document, reopened from the old file.
    When another `file` holds the destination with unsaved edits, `save_as` is refused with `conflict` before anything is written.
- Without `file`: the target is the document open in the running YoluPainter. Turn on "Accept external commands" in the app's settings first.

## Connecting

`yolupainter-cli.exe` is in `%LOCALAPPDATA%\Programs\YoluPainter\` when installed with the installer (next to `yolupainter.exe` in the zip). `<name>` below is your Windows user name.

### Claude Desktop (extension)

Download `yolupainter-<version>-x86_64-pc-windows-msvc.mcpb` from [Releases](https://github.com/YozoraKurage/YoluPainter/releases), double-click it or drag it onto the extensions page of Claude Desktop's settings,
and confirm the installation. The extension contains `yolupainter-cli.exe`, so working with `file` is possible even without installing YoluPainter itself. To operate the app, install and start the app as well.
The extension is not replaced by app updates; install the new `.mcpb` the same way.

### Claude Code

```
claude mcp add yolupainter -- C:\Users\<name>\AppData\Local\Programs\YoluPainter\yolupainter-cli.exe mcp
```

To share it in a project, write it in `.mcp.json`:

```json
{
  "mcpServers": {
    "yolupainter": {
      "command": "C:\\Users\\<name>\\AppData\\Local\\Programs\\YoluPainter\\yolupainter-cli.exe",
      "args": ["mcp"]
    }
  }
}
```

### ChatGPT desktop (Work and Codex)

Local servers on standard input and output work in the desktop Work and Codex. Add this to `~/.codex/config.toml` (single quotes let you write a Windows path as it is):

```toml
[mcp_servers.yolupainter]
command = 'C:\Users\<name>\AppData\Local\Programs\YoluPainter\yolupainter-cli.exe'
args = ["mcp"]
```

ChatGPT on the web and claude.ai can only reach servers published over HTTPS, so this local server cannot be used there (not supported for now).
ChatGPT may not display MCP image replies. Besides the image, a preview is also returned as a link (the same PNG can be read with `resources/read`).

## Settings in YoluPainter

Turning on "Accept external commands" in the app's settings makes the app accept commands from programs of the same user on the same PC. It is off by default, and a small mark appears in the status bar while it is on.
Turning it off stops accepting and closes the connections. While it is off, tools without `file` return a "cannot connect" error that says how to fix it.

The app refuses with a reason while you are drawing, while a save is running, for read-only sets and so on. A command it accepts becomes one undo step of the app.

## Tools and resources

The list of tools and the arguments and kind of each are in [Commands](CLI.md#commands) of the command-line document. Each tool has a name, a title, a description, JSON Schemas for input and output,
and annotations saying whether it only reads, whether it is destructive and whether repeating it with the same arguments is harmless. A reply is structuredContent (matching the output JSON Schema) and a text with the same content.
A failure is an `isError` reply holding JSON with `code`, a Japanese and an English `message`, and `data`.

- A preview (`preview`) returns the PNG as an image and also a resource_link (`yolupainter://preview/<number>.png`; the last 8 are kept) to the same PNG.
- Resources: `yolupainter://docs/<name>` is the documentation of the installed version (`guide`, `cli`, `mcp`, `install`, `psd`, `brush`, the .ylp format specification `ylp-format` and so on; where an English version exists it is the default, and `<name>.ja` and `<name>.en` choose a language).
  `yolupainter://ops/commands` is the command list with JSON Schemas, and `yolupainter://ops/effect-kinds` is the effect kinds and their value ranges (the same as `effect_list_kinds`).
- Protocol versions 2025-11-25 and 2026-07-28 are both answered (the flow with `initialize`, and the flow with `server/discover` and a per-request `_meta`).

## Safety

- Destructive operations (deleting layers, masks and effects, saving over a file, replacing an existing file) are refused without changing anything unless the argument `confirm: true` is given, and the tools carry the destructive annotation.
  The AI should confirm with the user before passing `confirm: true`. Saving keeps the previous version in the `<file name>-backups~` folder next to the file.
- There is no tool that runs arbitrary code. The files touched are the `.ylp` of `file` and the paths named by the export and save tools. A relative path cannot climb out of the working folder with `..`.
- Commands to the app are accepted only from programs of the same user on the same PC. The channel is a Unix socket or a Windows named pipe; a key file is placed in a folder only that user can read, and each connection proves it knows the key.
  The name and key are separate from Live Link's. No network port is opened.
- Whether an MCP client trusts tool annotations is up to the client. In the assistant's settings it is safer to make destructive tools ask every time.

## When it does not connect

- "Cannot reach a running YoluPainter": the app is not running, or "Accept external commands" is off. Turn it on, or pass `file` to work without the app.
- "No reply": the app is busy (for example drawing) and did not take the command. It is unknown whether the command ran, so check with `doc_info` or `history_info` before asking again.
