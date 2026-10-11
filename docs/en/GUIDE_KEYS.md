# Keyboard Shortcuts

[日本語](../GUIDE_KEYS.md)

A list of keyboard, mouse and pen controls. You can view and change the keys and mouse combinations in the Keyboard Shortcuts category of Edit → Settings… in the app (see "Change shortcuts" below). The tables under "Default assignments" below are made from the app's table and show the assignments before any change. On Mac, read `Ctrl` in these tables as `Command` (menus show `Cmd` too). The one exception is `Ctrl+Tab` for the mode pie menu, which is `Control` on Mac as well.

## Change shortcuts

Press Keyboard Shortcuts on the left of Edit → Settings… (`Ctrl+,`) to view the keys and mouse combinations and change them. Choose one of the sections listed below it (Everywhere & View / Paint / Edit / Pose / During an Operation / Pie Menus); the table on the right shows each action's name, keys and mouse combination.

| Action | Input |
|---|---|
| Change a key | Click a key cell and press the key you want (modifier keys alone do not set it). The `+` cell adds another key. Keys that work while held (such as `Space`) and keys during an operation are a single key without modifiers |
| Cancel / remove the key | `Esc` / `Backspace` while it waits |
| Change a mouse combination | Click a mouse cell, then press the button you want (left, right, middle, back or forward) on that cell while holding the modifier keys. Both the button and the modifiers change. The row for the stencil rotation steps changes only its modifiers (a press without modifiers does not set it) |
| Search by name / find by key | Type a name in the field above the table / click "Find by Key" and press the key (only rows with that key are shown, from every section; click again to show all). The Search Settings field at the very top of the Settings window searches the other settings and does not narrow this table |
| Reset one row / reset everything | The mark at the right end of a changed row / "Reset to Default" at the bottom |
| Export / import | "Export…" / "Import…" at the bottom (keys, mouse combinations and pie menus together. Importing replaces the current changes; unknown actions are skipped and their number is reported) |

- When the same key points to different actions in the same section and situation, the rows turn red and the tooltip names the other action. While a conflict remains, the changed assignments work but are not saved to the file (the bottom bar shows the reason).
- A key in a mode section (Paint, Edit, Pose) taking the same key before the Everywhere section is not a conflict (the tooltip names the action that takes it first).
- For the mouse, putting a 2D or 3D view action (including one done on releasing without moving) on the left button without modifiers conflicts with painting. The 2D canvas looks at a press for the view actions first, then the selection combinations, then the eyedropper, so besides the same combination, giving a later action an earlier action's combination with extra modifiers is also a conflict (for example, with rotation on `Alt` + left button, making selection Add `Alt+Shift` + left button).
- A mouse combination also matches when you hold modifiers it does not name (rotation on `Alt` + left button also rotates with `Alt+Shift` + left button). Actions done on releasing without moving and the 2D eyedropper match only the modifiers as written.
- The clipboard keys (copy, cut, copy merged, paste), the fixed roles of `Esc`, `Enter` and `Backspace`, and the left click that selects objects in Edit mode cannot be changed.
- Changes are saved in `keymap.json` in the settings folder, and only what differs from the defaults is written. If `keymap.json` cannot be read, the app starts with the default keys and says so, and the file is kept (it is moved to `keymap.broken.json` before the next save; if that name is taken, a number is added, as in `keymap.broken-2.json`).
- In the Pie Menus section, choose a pie and set its name, the key that opens it, and the items in its 8 places (a command, another pie, a yolu-ops command or a recorded action). `+` adds a pie and the trash mark deletes one (the built-in Mode and View pies can only have their items changed).
- The keys shown in menus and tooltips follow the changed assignments (only keys that work in the current mode are shown).
- For a tool key (the tool rows in the Paint section), choose how it behaves in the box to the right of the row (the heading reads "Mouse / Behavior"), key by key.
  - **Switch on Press** (default): pressing the key switches to the tool. Releasing does not switch back.
  - **Only While Held**: the tool is active only while the key is held; on release the previous tool (and its brush settings) comes back.
  - **Tap to Switch, Hold for Temporary**: release within 0.25 seconds and the tool stays. Release after more than 0.25 seconds, or after drawing while the key was held, and the previous tool comes back.
  - If you release in the middle of a stroke, the stroke (or a selection shape, gradient, shape or ruler being dragged) is finished before the tool returns (2D and 3D, mouse and pen; a polygon selection keeps the tool until it is closed or cancelled). Pressing the same key again during the same stroke simply continues. If you choose a tool or brush on the toolbar, with another key, in a pie menu or in the brush list while the key is held, the tool is not switched back and your choice stays. Pressing another only-while-held key stacks: releasing the later key returns to the earlier key's tool, and releasing the earlier key first returns to the original tool when the later key is released. Changing the mode counts as releasing the key: the previous tool comes back before the mode changes. Losing window focus, opening a menu or dialog, or starting to type in a text field counts as releasing the key. Pressing the key of the tool that is already selected changes nothing. While the key is held, a dot marks the button of the tool that will come back. A key that includes a modifier such as `Shift` lasts until the key itself is released, even if the modifier is released first.

## Modes and pie menus

There are three modes: Paint, Edit and Pose. You paint only in Paint mode. The tool keys, the brush size (`[` / `]`), swapping colors and the default colors (`X` / `D`), the keys for paths and point gradients, Quick Mask (`Shift+Q`), the arrow keys of Move / Transform, screen color picking (Windows), and the keys that change pixels (`Delete` for Erase Selection, `Ctrl+X` for Cut, `Ctrl+V` for Paste, `Ctrl+E` for Merge Down and `Ctrl+Shift+E` for Merge Visible) work only in Paint mode. Menu items and buttons work in every mode (menus show only the keys that work in the current mode).

| Action | Input |
|---|---|
| Mode pie menu | `Ctrl+Tab` (opens at the pointer. It does not open while a mouse button is held or during a drag) |
| Choose a pie menu item | Keep the key held, move toward an item, and release. If you release right away, the pie stays open: click an item. `1`–`8` (clockwise from the top) also choose |
| Close a pie menu | `Esc`, a right click, or a click in the middle. Holding the key for a while and releasing it without moving also closes it |

## Edit and Pose modes

In Edit mode, the white markers in the 3D view select fill projection boxes and decals, gradient decals, filter shapes, points of point gradients, 3D paths and 3D rulers, and `G`, `R` and `S` move them. In Pose mode they move the selected bone. Nothing can be moved on the 2D canvas (it is for viewing only).

| Action | Input |
|---|---|
| Select an object (Edit) | Click its marker in the 3D view (its layer is selected too). For overlapping markers, click the same place again for the next object. Click an empty spot to deselect |
| Move / rotate / scale | `G` / `R` / `S` (starts at the pointer and follows how far the mouse moves. Points can only be moved). Choosing Move, Rotate or Scale in the tool strip and dragging with the left button from a marker (in Pose mode, from the selected bone's surface) does the same; releasing applies it. A pen works too |
| Choose an axis | `X` / `Y` / `Z` (press again for the object's own axis, a third time for no axis). `Shift+X` / `Shift+Y` / `Shift+Z` lock the plane without that axis. When scaling a projection box, decal, shape or bone, the axis is the object's own axis from the first press (press again for no axis) |
| Type a value | Digits, `.`, `-` (flips the sign) and `Backspace` |
| Apply / cancel | Left click or `Enter` / right click or `Esc` (cancelling restores the state before you started and leaves nothing in the undo history) |
| Snap | Hold `Ctrl` (by default move 0.1, rotate 5°, scale 0.1; change them in the options bar). `Shift+Tab` snaps all the time (then `Ctrl` does the opposite) |
| Reset position / rotation / scale | `Alt+G` / `Alt+R` / `Alt+S` |
| Hide the selected object's marker / show all (Edit) | `H` / `Alt+H` |
| Delete the selected object (Edit) | `Delete` (projection boxes and decals, and the last point of a point gradient, cannot be deleted. Holding the key deletes only one) |
| Hide / show the shape handles (Edit) | `Q` |

Each move, rotation or scale is one undo step (one pose undo step for bones). A 3D path is placed back onto the nearest surface points when you apply (only on surfaces facing the same way as where it was drawn, so a path on the front of a thin plate does not move to the back). If a point is too far from the surface, it cannot be applied: keep moving or cancel. Objects on hidden layers (including layers in a hidden group) cannot be selected or moved.

## Modifiers while painting

| Action | Input |
|---|---|
| Draw a straight line from the end of the previous stroke | `Shift`-click with the brush or eraser (when there is no previous point, hold `Shift` and drag to lock the direction to 45° steps; in 2D and 3D, with the mouse and the pen) |
| Set the clone source | `Alt`-click with the Clone brush (release without moving; in 2D and 3D. If you move, 2D rotates the view and 3D snap-orbits) |
| Pick a color with any tool (eyedropper) | In 2D, press the right button (while it is held, the swatch at the pointer follows it; the color where you release is picked; `Esc` cancels). In 3D, release the right button without moving it (if you move it, the view just orbits). With Polygon Fill, the right button opens the island menu |
| Shape: 45° steps, square or circle / draw from the center | `Shift` / `Alt` pressed after starting to drag |
| Turn Snap to Ruler on or off | `Ctrl+1` |
| Turn Snap to Special Ruler on or off | `Ctrl+2` |
| Switch the snapping special ruler | `Ctrl+4` (steps through the special rulers of the space of the view the pointer is over; if it is over neither view, of the view where you last started drawing with a tool; if there is none, 3D when you can only paint in the 3D view, and 2D otherwise) |
| Cancel the current stroke, shape or ruler drag, selection operation or path point drag | `Esc`. If there is nothing to cancel, it deselects the selected point, and if there is no point, it deselects the selection. An input field, menu or window that is using the key takes it first |

## View controls and `Alt`

View controls are the same in 2D and 3D. `Alt` is a view control (rotating the view in 2D, snap orbit in 3D) if it is held at the moment you press the button; if you press it after pressing the button, it is the tool's modifier (From center for shapes, rectangles and ellipses, breaking a path handle, and so on). A drag of a selection or shape that starts with `Alt` held becomes a view rotation.

These combinations no longer work. In 3D, `Shift` + right-drag and `Alt` + `Shift` + left-drag (move; with `Shift` held, a right-drag orbits and an `Alt` + left-drag snap-orbits). In 2D, `R` + left-drag and `Shift` + middle-drag (rotate; with `Shift` held, a middle-drag pans). In 2D, `Alt`-click as a temporary eyedropper (`Alt` + left-drag now rotates the view, and the eyedropper moved to the right button). In 3D, `Alt` + left-drag as a free orbit (it is now the snap orbit).

## Situational keys and notes

- With a selection tool, holding `Shift` (add), `Ctrl` (subtract) or `Shift+Ctrl` (intersect) when you start pressing changes how the selection is made (also on the options bar; with the Selection Pen, `Shift` gives the pen and `Ctrl` gives the eraser). `Shift` pressed after starting fixes the ratio, and `Alt` draws from the center.
- Polygon Select closes with `Enter` (or a click on the first point, or a double-click), and `Backspace` removes its last point. Move / Transform applies with `Enter` and cancels with `Esc`; the arrow keys move by 1 pixel (10 with `Shift`).
- `Ctrl+J` copies the selection to a new layer when there is a selection, and duplicates the layer otherwise.
- With the Path tool, `Delete` / `Backspace` delete the selected point (or the last point if none is selected). While a point of a point gradient is selected, they delete that point.
- In the layer list, `Ctrl`-click toggles, `Shift`-click selects a range, and `Ctrl+Shift`-click adds a range.
- The wheel also zooms. `Ctrl+Space` + left-drag zooms as you move horizontally, around the pressed point (releasing without moving zooms in, and adding `Alt` zooms out).
- Rotating the 2D view goes in 15° steps, and is free with `Shift` also held. The 3D snap orbit snaps to an axis view (front, back, right, left, top, bottom) when the direction comes within 15° of it.
- Rotating the view right uses the `^` character, and the `=` key also works (keyboard layouts put the keys in different places).
- While the right button is held in the 3D view, `W` / `S`, `A` / `D` and `Q` / `E` move the view forward / back, left / right and down / up (`Shift` for faster; meanwhile these keys are not used for switching tools and so on). In orthographic, `W` / `S` zoom in and out.
- The 3D axis views (Front, Back, Right, Left, Top, Bottom) and Toggle Orthographic are in the View Pie Menu and on the axis widget in the top right of the 3D view (neither has a key at first). Choosing an axis view or snapping onto an axis makes the view orthographic, and leaving the axis brings back perspective ("Axis views and orthographic" in [GUIDE_3D.md](GUIDE_3D.md)).
- In Edit mode `H` hides the selected object's marker, so flip the 2D view with View → Flip View.
- The stencil rotates with a left-drag while `Y` is held (15° steps with `Shift`), moves with a middle-drag or `Ctrl` + left-drag, and scales with a right-drag or `Alt` + left-drag (in 2D and 3D). It is not used while `N` is held.
- The Liquify tool has no default key.

## Pen

The side buttons act as the right button (in 2D they pick a color; in 3D they orbit, and releasing without moving picks a color; with Polygon Fill in 2D they do nothing). `Alt`, `Space` and `Ctrl+Space` act the same as with the left mouse button. Only a pen tip with neither `Ctrl` nor a side button pressed (and the eraser end) paints, and a pen tip with `Shift` pressed draws a straight line (in 2D and 3D).

## Default assignments

<!-- keymap:begin -->

### Everywhere & View

| Action | Key |
|---|---|
| Ungroup | `Ctrl+Shift+G` |
| Copy to a New Layer (With a selection) | `Ctrl+J` |
| Duplicate | `Ctrl+J` |
| Group Layers | `Ctrl+G` |
| Invert Selection | `Ctrl+Shift+I` |
| Redo | `Ctrl+Shift+Z` / `Ctrl+Y` |
| Select All | `Ctrl+A` |
| Deselect | `Ctrl+D` |
| Undo | `Ctrl+Z` |
| New Layer | `Ctrl+Shift+N` |
| Save As… | `Ctrl+Shift+S` |
| Save | `Ctrl+S` |
| Open… | `Ctrl+O` |
| New Project… | `Ctrl+N` |
| Snap to Ruler | `Ctrl+1` |
| Snap to Special Ruler | `Ctrl+2` |
| Switch the Snapping Special Ruler | `Ctrl+4` |
| Fit to Screen | `Ctrl+0` |
| Zoom In | `Ctrl++` / `Ctrl+=` |
| Zoom Out | `Ctrl+-` |
| Quit | `Ctrl+Q` |
| Settings… | `Ctrl+,` |
| Reset Rotation | `Shift+R` |
| Mode Pie Menu | `Ctrl+Tab` |
| Flip View | `H` |
| Rotate View Left | `-` |
| Rotate View Right | `^` / `=` |
| Copy Merged | `Ctrl+Shift+C` |
| Copy | `Ctrl+C` |
| Pan / Ctrl: Zoom | `Space` |
| Frame Selected Set | `.` |
| Cancel Operation / Deselect | `Esc` |
| Confirm Transform / Polygon Selection | `Enter` |
| Delete Last Polygon Selection Point | `Backspace` |
| Cancel Operation | `Esc` |

### Paint

| Action | Key |
|---|---|
| Merge Visible | `Ctrl+Shift+E` |
| Merge Down | `Ctrl+E` |
| Delete Gradient Point (With a point selected) | `Delete` / `Backspace` |
| Erase Selection (With a selection) | `Delete` |
| Brush | `B` |
| Eraser | `E` |
| Fill | `G` |
| Gradient | `Shift+G` |
| Shape | `U` |
| Ruler | `Shift+U` |
| Polygon Fill | `4` |
| Eyedropper | `I` |
| Rectangle Select | `M` |
| Ellipse Select | `Shift+M` |
| Lasso | `L` |
| Polygon Select | `Shift+L` |
| Magic Wand | `W` |
| ID Color Select | `Shift+W` |
| Selection Pen | `S` |
| Move / Transform | `V` |
| Path | `P` |
| Text | `T` |
| Quick Mask | `Shift+Q` |
| Delete Path Point (Path) | `Delete` / `Backspace` |
| Finish Path (Path) | `Enter` |
| Projection Handles | `Q` |
| Swap Main and Sub Colors | `X` |
| Default Colors | `D` |
| Smaller Brush | `[` |
| Larger Brush | `]` |
| Hide Window and Pick Screen Color (Windows) | `Ctrl+Alt+Shift+I` |
| Pick Screen Color (Windows) | `Ctrl+Alt+I` |
| Cut | `Ctrl+X` |
| Paste | `Ctrl+V` |
| Stencil | `Y` |
| Bypass Stencil | `N` |
| Move ← (1 px) (Move / Transform) | `←` |
| Move → (1 px) (Move / Transform) | `→` |
| Move ↑ (1 px) (Move / Transform) | `↑` |
| Move ↓ (1 px) (Move / Transform) | `↓` |
| Move ← (10 px) (Move / Transform) | `Shift+←` |
| Move → (10 px) (Move / Transform) | `Shift+→` |
| Move ↑ (10 px) (Move / Transform) | `Shift+↑` |
| Move ↓ (10 px) (Move / Transform) | `Shift+↓` |

### Edit

| Action | Key |
|---|---|
| Move Selected | `G` |
| Rotate Selected | `R` |
| Scale Selected | `S` |
| Reset Position of Selected | `Alt+G` |
| Reset Rotation of Selected | `Alt+R` |
| Reset Scale of Selected | `Alt+S` |
| Toggle Snapping | `Shift+Tab` |
| Hide Selected Marker | `H` |
| Reveal Hidden Markers | `Alt+H` |
| Delete Selected | `Delete` |
| Projection Handles | `Q` |

### Pose

| Action | Key |
|---|---|
| Move Selected | `G` |
| Rotate Selected | `R` |
| Scale Selected | `S` |
| Reset Position of Selected | `Alt+G` |
| Reset Rotation of Selected | `Alt+R` |
| Reset Scale of Selected | `Alt+S` |
| Toggle Snapping | `Shift+Tab` |

### During an Operation

| Action | Key |
|---|---|
| Move Forward (While Right Button Held) | `W` |
| Move Back (While Right Button Held) | `S` |
| Move Left (While Right Button Held) | `A` |
| Move Right (While Right Button Held) | `D` |
| Move Down (While Right Button Held) | `Q` |
| Move Up (While Right Button Held) | `E` |

### Mouse

| Action | Combination | Mode |
|---|---|---|
| Pan (2D) | Middle Button | Everywhere & View |
| Rotate (2D) | Alt+Left Button | Everywhere & View |
| Set Clone Source (2D) | Alt+Left Button Released without Moving | Paint |
| Eyedropper (2D) | Right Button | Everywhere & View |
| Intersect with Selection (Selection) | Ctrl+Shift+Left Button | Paint |
| Add to Selection (Selection) | Shift+Left Button | Paint |
| Subtract from Selection (Selection) | Ctrl+Left Button | Paint |
| Orbit (3D) | Right Button | Everywhere & View |
| Eyedropper (3D) | Right Button Released without Moving | Everywhere & View |
| Pan (3D) | Middle Button | Everywhere & View |
| Zoom (3D) | Space+Ctrl+Left Button | Everywhere & View |
| Pan (3D) | Space+Left Button | Everywhere & View |
| Snap Orbit (3D) | Alt+Left Button | Everywhere & View |
| Set Clone Source (3D) | Alt+Left Button Released without Moving | Paint |
| Move Stencil (Stencil) | Y+Middle Button | Paint |
| Move Stencil (Stencil) | Y+Ctrl+Left Button | Paint |
| Scale Stencil (Stencil) | Y+Right Button | Paint |
| Scale Stencil (Stencil) | Y+Alt+Left Button | Paint |
| Rotate Stencil (Stencil) | Y+Left Button | Paint |
| Snap Stencil Rotation to 15° (Stencil) | Y+Shift+Left Button | Paint |
<!-- keymap:end -->

## If you get lost

- If you lose the whole model, use View → Frame the Model in the 3D View.
- To put the panels back, use Window → Reset Panel Layout.
- You can also open a `.ylp` by dragging it onto the window.
