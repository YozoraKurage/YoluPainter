# Getting Started

[日本語](../GUIDE_START.md)

This page covers the screen layout, creating and opening projects, your first stroke, moving the 2D view, and arranging panels. When YoluPainter starts, it opens a 2048 × 2048 canvas and a demo cube.

## The screen

| Area | Contents |
|---|---|
| Menu bar (top) | File, Edit, Layer, Select, Filter, View, Window and Help. At the right end are the project name (a “•” marks unsaved changes) and the Unity mark, the entrance to Live Link |
| Options bar (below it) | The mode (Paint, Edit or Pose) at the left end. To its right, two or three settings you use most with the selected tool. They are the same values as Tool Properties |
| Toolbar (left edge) | The tool buttons (in Edit and Pose modes, the four tools Select, Move, Rotate and Scale). The main and sub colors sit at the bottom |
| Left dock | From the top, stacked: Tools (the sub tools), Tool Properties and Brush Size, with Color and Color Sets at the bottom |
| Center | The 3D View on the left and the Canvas (2D) on the right, side by side. Both are tabs, so you can also stack them and switch |
| Right dock | From the top: Texture Sets, Channels and Assets; then Layers and Log; then Properties, Material and History. The Navigator is closed; open it from the Window menu and it joins the Texture Sets group (it is checked while open, and choosing it again closes it; the Actions panel works the same way) |
| Status bar (bottom) | The version and build (for example `0.6.0 · a1b2c3d`) and the memory in use at the right end. The left is empty; the result of your last operation appears as a small notice just above the bar |

The Tools, Tool Properties and Brush Size panels change with the selected tool. Brushes and their settings are in [GUIDE_PAINT.md](GUIDE_PAINT.md). Properties shows the contents of the selected layer ([GUIDE_LAYERS.md](GUIDE_LAYERS.md)). Material shows the look settings of the current texture set ([GUIDE_3D.md](GUIDE_3D.md)). The Pose panel appears when you load a model with bones ([GUIDE_3D.md](GUIDE_3D.md)).

## Modes

You paint in Paint mode. Edit and Pose modes do not paint: the 2D canvas is for viewing only (moving, rotating and zooming the view, and the right-button eyedropper still work), and nothing is painted in the 3D view either. The brush and color panels are dimmed and cannot be used. Pose mode can be chosen only while a model with bones is loaded ([GUIDE_3D.md](GUIDE_3D.md)). In Edit mode, the markers in the 3D view select fill projections, gradient decals, filter shapes, points, 3D paths and 3D rulers, and `G`, `R` and `S` move them; in Pose mode the same keys move the selected bone ("Select and move objects" in [GUIDE_3D.md](GUIDE_3D.md)).

Switch modes with any of the following. The mode cannot change while you are drawing.

- The dropdown at the left end of the options bar.
- The pie menu on `Ctrl+Tab` (it opens at the pointer. Keep the key held, move toward an item and release, or release right away and click an item. See [GUIDE_KEYS.md](GUIDE_KEYS.md)).
- View → Mode.
- Choosing a painting tool (for example from the Edit menu) switches to Paint, and choosing a bone in the "Bones" tree of the Pose panel switches to Pose.

## Creating, opening and saving

### Create a new project

Choose File → New Project… (`Ctrl+N`), set the following, and press Create.

- Template: “PBR (all channels)”, “lilToon (Color, Normal, Emission)” or “Color only”. This sets which channels the first layer uses.
- Mesh: “Choose a model…” lets you pick an FBX, and you can make a texture set for each material of the model. If you do not pick one, you paint in 2D and can choose a model later.
- Resolution: 512, 1024, 2048, 4096 or 8192.
- Normal map format, and “Bake mesh maps after creating”.

To change a project afterwards, use File → Project Configuration…. It adds and removes texture sets, renames them, changes their materials and sizes, replaces or reloads the model, and changes the normal map format.

### Texture sets

A texture set is the picture that goes on one material of the model. A project holds up to 64 of them. Layers, undo history and selections belong to each texture set. In the Texture Sets panel you can switch sets, rename one (double-click), add an empty set (same size and channels as the current one), remove the current set, and hide or show a set in the 3D view and Unity (the eye icon).

### Open and save

File → Open… (`Ctrl+O`) opens a `.ylp`. You can also drop a `.ylp` onto the window. If there are unsaved changes, you are asked whether to discard them. Dropping an FBX loads it into the 3D view, and dropping a PSD imports it as a new texture set.

Save with Save (`Ctrl+S`) or Save As… (`Ctrl+Shift+S`). Exporting, PSD, Save for Distribution and recovery are in [GUIDE_FILES.md](GUIDE_FILES.md). For exchanging `.ylp` files with the Unity version, see [UNITY.md](UNITY.md).

## Your first stroke

1. Pick a color: the main color at the bottom of the toolbar, or the Color panel.
2. Pick a brush in the Tools panel (the key is `B`).
3. Select the layer to paint on in the Layers panel. To make a new one, press `Ctrl+Shift+N`.
4. Left-drag on the Canvas or the 3D View.
5. Undo with `Ctrl+Z`, redo with `Ctrl+Shift+Z` (or `Ctrl+Y`). In the History panel, clicking a row goes straight back to that step.
6. Save with `Ctrl+S`.

## Moving the 2D view

These change only the view. The saved image is never rotated or flipped.

| Action | Input |
|---|---|
| Zoom | Wheel, or hold `Ctrl+Space` and left-drag (move left and right; the pressed point is the center) |
| Pan | Middle-drag, or hold `Space` and left-drag |
| Rotate | Hold `Alt` and left-drag (15° steps; free if `Shift` is also held). `Shift+R` resets it |
| Flip horizontally | `H` |
| Fit to the view | `Ctrl+0` |

From the top, the View menu has the UV Wireframe and Overlapping UVs switches ([GUIDE_3D.md](GUIDE_3D.md)), Zoom In, Zoom Out, Fit to Screen, (separator) Rotate View Left, Rotate View Right, Reset Rotation, Flip View, (separator) Snap to Ruler, Snap to Special Ruler and Switch the Snapping Special Ruler ([GUIDE_PAINT.md](GUIDE_PAINT.md)), (separator) Mode and Frame the Model in the 3D View. In the Navigator panel, press or drag in the thumbnail of the whole canvas to move the view (the current view is outlined). It also has Zoom and Rotation sliders and buttons for Flip horizontally, Fit canvas, 100% and Reset rotation. For the 3D view, see [GUIDE_3D.md](GUIDE_3D.md).

## Arranging panels

Drag a tab to put it in another group of tabs, or drop it at an edge to place panels side by side. With the Canvas and the 3D View side by side, you can paint while watching both. The arrangement (tab groups, splits, sizes and which tab is in front) and the window's size, position and maximized state are remembered in `YoluPainter/layout.json` in the settings folder and restored at the next start. As long as you have not moved a divider or a tab, the width of the right column follows the window width (about 19% of it, with a lower limit); once you have, it stays as you set it. If the file cannot be read, has an unknown tab, or lacks a tab or repeats one, the app starts with the default arrangement. The exception is Tool Properties, Brush Size and Material, which an arrangement saved by an earlier version does not have: they are added when the file is read instead of discarding the arrangement (Tool Properties and Brush Size as a column below the Tools group, Material right after Properties), and nothing else in the arrangement changes (in an arrangement from an earlier version, Assets, Channels and the Navigator also stay in their old group, and the Canvas and the 3D View stay as one group of two tabs.). 0.5.x starts with the default arrangement when it reads a file that has these three tabs, because it does not know them. If the saved position is not visible on any connected screen, only the size is restored. A window larger than the screen is fitted to it once after starting. 0.4.x starts with the default arrangement when it reads this file with separate windows in it.

The Window menu lists the panels. A panel that is open is marked, and choosing it brings it to the front; choosing a closed panel opens it. Reset Panel Layout restores the default arrangement and closes separate windows.

### Separate windows

Right-click a dock tab and choose Open in New Window, or drag the tab outside the app window and release it. The panel becomes a separate OS window that can sit on another monitor. The Canvas and the 3D View can paint in separate windows too. Shortcuts, pen pressure and file drops work the same in whichever window has focus.

- Tabs can be dragged between separate windows to group them.
- To return a panel to the dock, release its tab over the app window's dock, choose Return to Dock from its context menu, or close the window. Closing returns it to the group it came from.
- Position, size and tabs come back at the next start. A window that was on a monitor that is no longer connected opens over the app window (Windows; Linux reopens it at the remembered position).
- On Windows, separate windows do not appear in the taskbar, stay in front of the app window, and hide with it when it is minimized. They have no OS title bar: drag an empty part of the tab row to move one, double-click it to maximize or restore, use the close button at the right end of the row to return its panels to the dock, and drag an edge to resize.
- You cannot drag an item from Assets and drop it onto a panel in a separate window. In a separate window's panels, the pen's side button does not act as a right click (it works on the Canvas and the 3D View).

### The title bar on Windows

On Windows the app does not use the OS title bar. Minimize, maximize (restore while maximized) and close sit at the right end of the menu bar. Drag an empty part of the bar to move the window, double-click it to maximize or restore, and drag an edge to resize. Mac and Linux keep the OS frame.

### Scrolling lists

Lists and panels scroll the same way everywhere. Use the wheel, or grab the handle at the right edge. Pressing the groove moves the handle to that point, and you can keep dragging. A pen's press-and-drag moves the handle just as a mouse does. The handle is thin normally and thickens when you bring the pointer near or grab it.

## Notices and the log

The result of an operation, or the reason it could not be done, appears as a small notice just above the status bar and then fades.

- A notice that something was done stays for 4 seconds, a refusal, warning or failure for 12. Click it to dismiss it at once. It stays while the pointer is on it (even then, it goes after 3 times as long: 12 and 36 seconds).
- A refusal or failure is one sentence: what happened (and why).
- The Log panel lists warnings and errors since the app started, with the time, kind, source and message (up to 1,000 lines; repeated notices are folded into one line with a count such as “×3”). The buttons at the top show errors only, copy the selected lines, copy all lines, and clear the log. Click a row to select it, `Ctrl`+click to add or remove, `Shift`+click for a range. Notices that something was done, and refusals, are not in the log. Help → Open Log Folder opens the folder with the diagnostic files.
- If there is a crash record you have not read yet, a “!” appears in the menu bar. It opens the Crash Report window, where you can copy the report, open the folder, or open the GitHub issue page. Nothing is sent automatically.

Hover over the memory figure at the right end of the status bar to see the breakdown (the whole app, layer memory, undo history, GPU). The GPU figure appears only when it can be obtained. While external commands are accepted, a small dot appears to its left ([GUIDE_SETTINGS.md](GUIDE_SETTINGS.md)).

## Where to go next

- Painting: [GUIDE_PAINT.md](GUIDE_PAINT.md)
- Selection and transform: [GUIDE_SELECT.md](GUIDE_SELECT.md)
- Layers and channels: [GUIDE_LAYERS.md](GUIDE_LAYERS.md)
- Keyboard shortcuts: [GUIDE_KEYS.md](GUIDE_KEYS.md) (also in the app under Edit → Settings… → Keyboard Shortcuts)
