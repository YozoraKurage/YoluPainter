# Getting Started

[日本語](../GUIDE_START.md)

This page covers the screen layout, creating and opening projects, your first stroke, moving the 2D view, and arranging panels. When YoluPainter starts, it opens a 2048 × 2048 canvas and a demo cube. The first layers are a white solid fill layer ("Fill 1") at the bottom and an empty "Layer 1" above it, and the empty layer is selected.

## The screen

| Area | Contents |
|---|---|
| Menu bar (top) | File, Edit, Layer, Select, Filter, View, Window and Help. At the right end are the project name (a “•” marks unsaved changes) and the Unity mark, the entrance to Live Link |
| Options bar (below it) | The mode (Paint, Edit, Pose or Hide) at the left end. To its right, two or three settings you use most with the selected tool. They are the same values as Tool Properties |
| Toolbar (left edge) | The tool buttons (in Edit and Pose modes, the four tools Select, Move, Rotate and Scale; in Hide mode, the buttons that select and hide faces). The main and sub colors sit at the bottom |
| Left dock | From the top, stacked: Tools (the sub tools), Tool Properties and Brush Size, with Color and Color Sets at the bottom |
| Center | The 3D View on the left and the Canvas (2D) on the right, side by side. Both are tabs, so you can also stack them and switch |
| Right dock | From the top: Texture Sets, Channels and Assets; then Layers and Log; then Properties, Material and History. The Navigator is closed; open it from the Window menu and it joins the Texture Sets group (it is checked while open, and choosing it again closes it; the Actions panel works the same way) |
| Status bar (bottom) | The version and build (for example `0.7.0 · a1b2c3d`) and the memory in use at the right end. The result of your last operation appears as a one-line notice at the left end ("Notices and the log" below) |

The Tools, Tool Properties and Brush Size panels change with the selected tool. Brushes and their settings are in [GUIDE_PAINT.md](GUIDE_PAINT.md). Properties shows the contents of the selected layer ([GUIDE_LAYERS.md](GUIDE_LAYERS.md)). Material shows the look settings of the current texture set ([GUIDE_3D.md](GUIDE_3D.md)). The Pose panel appears when you load a model with bones ([GUIDE_3D.md](GUIDE_3D.md)).

## Modes

You paint in Paint mode. Edit, Pose and Hide modes do not paint: the 2D canvas is for viewing only (moving, rotating and zooming the view, and the right-button eyedropper still work), and nothing is painted in the 3D view either. The brush and color panels are dimmed and cannot be used. Pose mode can be chosen only while a model with bones is loaded ([GUIDE_3D.md](GUIDE_3D.md)). In Edit mode, the markers in the 3D view select fill projections, gradient decals, filter shapes, points, 3D paths and 3D rulers, and `G`, `R` and `S` move them; in Pose mode the same keys move the selected bone ("Select and move objects" in [GUIDE_3D.md](GUIDE_3D.md)). In Hide mode, you hide faces in each 3D view by UV island and by bone ("Hide faces (Hide mode)" in [GUIDE_3D.md](GUIDE_3D.md)).

Switch modes with any of the following. The mode cannot change while you are drawing.

- The dropdown at the left end of the options bar.
- The pie menu on `Ctrl+Tab` (it opens at the pointer. Keep the key held, move toward an item and release, or release right away and click an item. See [GUIDE_KEYS.md](GUIDE_KEYS.md)).
- View → Mode.
- Choosing a painting tool (for example from the Edit menu) switches to Paint, and choosing a bone in the "Bones" tree of the Pose panel switches to Pose.

## Creating, opening and saving

### Create a new project

Choose File → New Project… (`Ctrl+N`), set the following, and press Create.

- Template: “PBR (all channels)”, “lilToon (Color, Normal, Emission)” or “Color only”. This sets which channels the first layers use. Every texture set starts with a white solid fill ("Fill 1", Color channel only) at the bottom and an empty "Layer 1" above it, selected (with a template that does not use Color, there is no white fill, only the empty layer).
- Mesh: “Choose a model…” lets you pick an FBX, and you can make a texture set for each material of the model. If you do not pick one, you paint in 2D and can choose a model later.
- Texture Sets: one row for each texture set to create. When you pick a model, there is a row for each material. Each row sets a name, a size (512, 1024, 2048, 4096 or 8192) and materials. The material list lets you choose several materials; choosing one that another row has moves it from that row. A row left without a material is not created.
- Normal map format, and “Bake mesh maps after creating”.

To change a project afterwards, use File → Project Configuration…. With the same rows as New Project, it adds and removes texture sets, renames them, changes their materials (several are allowed) and sizes, replaces or reloads the model, and changes the normal map format.

### Texture sets

A texture set is the picture that goes on materials of the model. When a texture set is assigned to several materials, the faces of those materials share the same picture. A project holds up to 64 of them. Layers, undo history and selections belong to each texture set. In the Texture Sets panel you can switch sets, rename one (double-click), add a set (same size and channels as the current one; it starts with the white solid fill and an empty layer), remove the current set, and hide or show a set in the 3D view and Unity (the eye icon). The row of a set assigned to several materials lists the material names. A texture set can hold several groups of layers (layer sets) inside it ("Layer sets" in [GUIDE_LAYERS.md](GUIDE_LAYERS.md)).

### Open and save

File → Open… (`Ctrl+O`) opens a `.ylp`. You can also drop a `.ylp` onto the window. If there are unsaved changes, you are told "There are unsaved changes." "Review Changes" (selected at first, `Enter`) goes back to the app without opening, "Discard Changes" discards the changes and opens, and "Cancel" (`Esc`) stops. File → Open Recent lists up to 10 `.ylp` files you opened or saved, newest first (the full path shows when you point at one; "Clear Recent" empties the list; a file that cannot be found is not opened and is removed from the list). Dropping an FBX loads it into the 3D view; for PSDs and images (PNG, JPEG, TGA) you choose where they go in the "Import Files" window ([GUIDE_FILES.md](GUIDE_FILES.md)).

Save with Save (`Ctrl+S`) or Save As… (`Ctrl+Shift+S`). Exporting, PSD, Save for Distribution and recovery are in [GUIDE_FILES.md](GUIDE_FILES.md). For exchanging `.ylp` files with the Unity bridge up to 0.4.x, see [UNITY.md](UNITY.md).

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

Two buttons float in the top right of the canvas. The left one, "eye · UV · ▽", turns the UV wireframe on and off when you press the eye or the UV letters, and ▽ changes the line color and opacity. The right one is the paint channel button: pressing it opens a panel with three columns, "Texture Set", "Layer Set" and "Paint Channel". Clicking a row switches to that texture set, layer set or channel (the channel you paint; the eye on the right picks the channel shown on the 2D canvas), and the "+" in a column header adds a texture set or a layer set. They cannot be pressed while you are drawing.

## Arranging panels

Drag a tab to put it in another group of tabs, or drop it at an edge to place panels side by side. With the Canvas and the 3D View side by side, you can paint while watching both. The arrangement (tab groups, splits, sizes and which tab is in front) and the window's size, position and maximized state are remembered in `YoluPainter/layout.json` in the settings folder and restored at the next start. As long as you have not moved a divider or a tab, the width of the right column follows the window width (about 19% of it, with a lower limit); once you have, it stays as you set it. If the file cannot be read, has an unknown tab, or lacks a tab or repeats one, the app starts with the default arrangement. The exception is Tool Properties, Brush Size and Material, which an arrangement saved by an earlier version does not have: they are added when the file is read instead of discarding the arrangement (Tool Properties and Brush Size as a column below the Tools group, Material right after Properties), and nothing else in the arrangement changes (in an arrangement from an earlier version, Assets, Channels and the Navigator also stay in their old group, and the Canvas and the 3D View stay as one group of two tabs.). 0.5.x starts with the default arrangement when it reads a file that has these three tabs, because it does not know them. If the saved position is not visible on any connected screen, only the size is restored. A window larger than the screen is fitted to it once after starting. 0.4.x starts with the default arrangement when it reads this file with separate windows in it.

The Window menu lists the panels. A panel that is open is marked, and choosing it brings it to the front; choosing a closed panel opens it. Reset Panel Layout restores the default arrangement and closes separate windows.

### Hide the UI

Press `Tab` to hide the toolbar, the options bar, the panels that are not views, the tab titles of the views and the buttons inside the views, leaving only the menu bar and the views (the Canvas and the 3D View). Press `Tab` again to bring them back (you can change the key at "Hide or Show UI" in Keyboard Shortcuts). The status bar stays while the UI is hidden, and shows "The UI is hidden. Tab brings it back." on its right. The arrangement is not changed, so it comes back with the same layout and sizes. Panels in separate windows are not hidden. It cannot be switched while you are drawing.

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

Lists and panels scroll the same way everywhere. Use the wheel, or grab the handle at the right edge. Pressing the groove moves the handle to that point, and you can keep dragging. A pen's press-and-drag moves the handle just as a mouse does. Holding the right button and moving over a list (Layers, brushes, Assets, Properties, Channels and so on) scrolls the list with the pointer (the list you pressed keeps scrolling even if the pointer leaves the panel). Holding the right button and moving over the toolbar on the left selects the tools the pointer passes over. Releasing without moving opens the right-click menu as before. The handle is thin normally and thickens when you bring the pointer near or grab it.

## Notices and the log

The result of an operation, or the reason it could not be done, appears as a one-line notice at the left end of the status bar and then fades. A text too long to fit before the items at the right end is cut with "…", and the full text shows when you point at it.

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
