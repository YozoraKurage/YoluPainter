# Operating from an AI assistant (MCP)

[日本語](../MCP.md)

While "Accept external commands" is on in its settings, YoluPainter is an MCP server at `http://127.0.0.1:17347/mcp`. AI assistants (Claude Code, Codex,
Claude Desktop and others) connect there to read the layers, masks and effects of the document open in the app, change values, look at preview images, export and save.
The tools live in the app, so updating the app brings the new tools. The connection stays inside this PC and never goes onto the network.
The tools are the same 26 as the [command line](CLI.md) commands (the `.` in names becomes `_`). Painting (strokes, fills, selections), baking and opening another document are not available.

## YoluPainter settings

1. In the app, open Edit → Settings…, choose Live Link & Commands and turn on "Accept external commands" (off by default). It listens only while on, and a small dot appears at the right end of the status bar. The dot's tooltip shows the URL to connect to.
2. The port can be changed in "Port", shown below it while it is on (17347 by default; 1024 to 65535). When you change it, use the same port in the programs that connect.
3. When it cannot listen (another program uses the port, for example), the dot turns to the "cannot accept" color and its tooltip says why. Change the port or close that program, then turn the setting off and on again.

A command becomes one step of the app's undo history and shows up in the app right away. While drawing, while saving and in read-only texture sets, commands are refused with a reason (`busy`, `read_only`).
Turning the setting off stops listening. Requests still waiting for a reply get a "stopped accepting" error (a save that already started still finishes).

## Connecting

### Claude Code (plugin)

The YoluPainter plugin holds the MCP setting that connects to the app and a skill describing how to use the tools.

```
/plugin marketplace add YozoraKurage/YoluPainter
/plugin install yolupainter@yolupainter
```

To receive new versions of the plugin automatically, select `yolupainter` under Marketplaces in `/plugin` and turn on auto-update (auto-update is off by default for other people's marketplaces).
From a shell, use `claude plugin marketplace add YozoraKurage/YoluPainter` and `claude plugin install yolupainter@yolupainter`.

You can also add only the MCP server, without the plugin:

```
claude mcp add --transport http yolupainter http://127.0.0.1:17347/mcp
```

The plugin points at the default port. If you changed the port in the app, add your port with the `claude mcp add` above and turn off the plugin's `yolupainter` in `/mcp`.

### Codex (plugin)

```
codex plugin marketplace add YozoraKurage/YoluPainter
codex plugin add yolupainter@yolupainter
```

Every time Codex starts, it refreshes the marketplaces you added and reinstalls the plugins from them (run `codex plugin marketplace upgrade` to do it by hand).

Without the plugin, add it to `~/.codex/config.toml` (`codex mcp add yolupainter --url http://127.0.0.1:17347/mcp` does the same):

```toml
[mcp_servers.yolupainter]
url = "http://127.0.0.1:17347/mcp"
```

### Claude Desktop (extension)

Chats in Claude Desktop connect through an extension (`.mcpb`) that speaks over standard input and output. Download `yolupainter-<version>-x86_64-pc-windows-msvc.mcpb`
from [Releases](https://github.com/YozoraKurage/YoluPainter/releases), then double-click it or drag it onto the extensions page of Claude Desktop's settings and confirm the installation.
The extension only relays standard input and output to the app's endpoint (`yolupainter-cli mcp`); the tools are answered by the app. If you changed the port in the app, set the extension's "Port" to the same.
The extension asks the app when Claude Desktop starts it. If the app is not accepting commands at that moment the connection fails, so start the app, turn on the setting, and then turn the extension off and on again.

### Other clients

Clients that speak Streamable HTTP MCP can connect to `http://127.0.0.1:17347/mcp`. Clients that only use standard input and output can start
`yolupainter-cli mcp` (with the installer: `%LOCALAPPDATA%\Programs\YoluPainter\yolupainter-cli.exe`; in the experimental Mac zip: `YoluPainter.app/Contents/MacOS/yolupainter-cli`; add `--port number` if you changed the port).

## Tools and resources

The list of tools and the arguments and kind of each are in [Commands](CLI.md#commands) of the command-line document. Each tool has a name, a title, a description, JSON Schemas for input and output,
and annotations saying whether it only reads, whether it is destructive and whether repeating it with the same arguments is harmless. A reply is structuredContent (matching the output JSON Schema) and a text with the same content.
A failure is an `isError` reply holding JSON with `code`, a Japanese and an English `message`, and `data`.

- A preview (`preview`) returns the PNG as an image and also a resource_link (`yolupainter://preview/<number>.png`; the last 8 are kept) to the same PNG.
- Resources: `yolupainter://docs/<name>` is the documentation of the installed version (`guide`, the entry page of the user guide, and its pages `guide-start`, `guide-paint`, `guide-select`, `guide-layers`, `guide-fill`, `guide-paths`, `guide-3d`, `guide-files`, `guide-settings` and `guide-keys`; also `cli`, `mcp`, `install`, `psd`, `brush`, the .ylp format specification `ylp-format` and so on; where an English version exists it is the default, and `<name>.ja` and `<name>.en` choose a language).
  `yolupainter://ops/commands` is the command list with JSON Schemas, and `yolupainter://ops/effect-kinds` is the effect kinds and their value ranges (the same as `effect_list_kinds`).
- `doc_open` returns the document only for the file the app has open. Another file is refused with `unsupported` (the app does not switch documents).
- `$selected` in a layer field (`layer`, `above`, `parent`) refers to the layer selected in the app's current texture set. `$created:<n>` works only inside a run of several commands
  (a command-line batch or an action) and is refused for a single tool call ([relative references](CLI.md#relative-references)).
- `action_run` applies a list of commands that change layers, masks and effects (`{"commands": [{"command": "layer.add", "args": {...}}, ...]}`) as one undo step in the app.
  If a command is refused, everything is rolled back and the error tells which one (`data.index`, from 0). `$created:<n>` works inside the list.
- Protocol versions 2025-11-25 and 2026-07-28 are both answered (the flow with `initialize`, and the flow with `server/discover` and a per-request `_meta`). The endpoint is stateless
  and does not use `Mcp-Session-Id`.

## Safety

- Destructive operations (deleting layers, masks and effects, saving over the file, replacing existing files) change nothing and are refused unless the argument `confirm: true` is given,
  and those tools carry the destructive annotation (in `action_run`, each destructive command of the list needs its own `confirm: true`, and the tool carries the annotation). The assistant should ask you before passing `confirm: true`. Saving keeps the previous version in `<file name>-backups~` next to the file.
- There is no tool that runs arbitrary code. The only files touched are the paths given to the export and save tools. Relative paths start at the folder of the open project, and `..` cannot leave it.
- It listens on `127.0.0.1` (inside this PC) only. A request whose `Host` is not `127.0.0.1:<port>` or `localhost:<port>`, or whose `Origin` is present and not the same place
  (a web page that points another name at 127.0.0.1, or a page of another site), is refused. Up to 8 connections at once.
- There is no password. While the setting is on, programs of other accounts on this PC can connect too. Turn it off when you do not use it.
- Whether a client trusts the tool annotations is up to the client. Setting the assistant to ask before every destructive tool is the safe choice.

## When it does not connect

- "Cannot reach a running YoluPainter": the app is not running, "Accept external commands" is off, or the port differs. Check the URL in the tooltip of the dot in the app's status bar.
- The dot shows "cannot accept": read the reason in its tooltip. If another program uses the port, change the port in the app and use the same port in the clients.
- "Too many connections" (`busy`): the 8 connections the app takes at once are used up, for example by other clients. Wait a moment and ask again.
- "No reply": the app could not take the command, for example while you are drawing. It is unknown whether the command ran, so check with `doc_info` or `history_info` before asking again.
