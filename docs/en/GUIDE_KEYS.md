# Keyboard Shortcuts

[日本語](../GUIDE_KEYS.md)

A list of keyboard, mouse and pen controls. The app lists the same assignments under Help → Keyboard Shortcuts. On Mac, read `Ctrl` in these tables as `Command` (menus show `Cmd` too). The one exception is `Ctrl+Tab` for the mode pie menu, which is `Control` on Mac as well.

## Modes and pie menus

There are three modes: Paint, Edit and Pose. You paint only in Paint mode. The tool keys (the Tools section below), the brush size (`[` / `]`), swapping colors and the default colors (`X` / `D`), the keys in the Paths and fill points section, Quick Mask (`Shift+Q`), the arrow keys of Move / Transform, screen color picking (Windows), and the keys that change pixels (`Delete` for Erase Selection, `Ctrl+X` for Cut, `Ctrl+V` for Paste, `Ctrl+E` for Merge Down and `Ctrl+Shift+E` for Merge Visible) work only in Paint mode. Menu items and buttons work in every mode (menus show only the keys that work in the current mode).

| Action | Input |
|---|---|
| Mode pie menu | `Ctrl+Tab` (opens at the pointer. It does not open while a mouse button is held or during a drag) |
| Choose a pie menu item | Keep the key held, move toward an item, and release. If you release right away, the pie stays open: click an item. `1`–`8` (clockwise from the top) also choose |
| Close a pie menu | `Esc`, a right click, or a click in the middle. Holding the key for a while and releasing it without moving also closes it |

## Edit and Pose modes

In Edit mode, the white markers in the 3D view select fill projection boxes and decals, gradient decals, filter shapes, points of point gradients and 3D paths, and `G`, `R` and `S` move them. In Pose mode they move the selected bone. Nothing can be moved on the 2D canvas (it is for viewing only).

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

## Tools

| Tool | Key |
|---|---|
| Brush / Eraser | `B` / `E` |
| Fill / Gradient / Polygon Fill | `G` / `Shift+G` / `4` |
| Shape (line, rectangle, ellipse) / Ruler | `U` / `Shift+U` |
| Eyedropper | `I` |
| Rectangle Select / Ellipse Select | `M` / `Shift+M` |
| Lasso / Polygon Select | `L` / `Shift+L` |
| Magic Wand / Selection Pen / ID Color Select | `W` / `S` / `Shift+W` |
| Move / Transform | `V` |
| Path | `P` |
| Text | `T` |
| Smaller / larger brush | `[` / `]` |
| Swap main and sub colors / default colors | `X` / `D` |

The Liquify tool has no key.

## Modifiers while painting

| Action | Input |
|---|---|
| Draw a straight line from the end of the previous stroke | `Shift`-click with the brush or eraser (when there is no previous point, hold `Shift` and drag to lock the direction to 45° steps; in 2D and 3D, with the mouse and the pen) |
| Set the clone source | `Alt`-click with the Clone brush (release without moving; in 2D and 3D. If you move, 2D rotates the view and 3D snap-orbits) |
| Pick a color with any tool (eyedropper) | In 2D, press the right button (while it is held, the swatch at the pointer follows it; the color where you release is picked; `Esc` cancels). In 3D, release the right button without moving it (if you move it, the view just orbits). With Polygon Fill, the right button opens the island menu |
| Shape: 45° steps, square or circle / draw from the center | `Shift` / `Alt` pressed after starting to drag |
| Turn Snap to Ruler on or off | `Ctrl+1` |
| Cancel the current stroke, shape or ruler drag, selection operation or path point drag | `Esc`. If there is nothing to cancel, it deselects the selected point, and if there is no point, it deselects the selection. An input field, menu or window that is using the key takes it first |

## Paths and fill points

| Action | Key |
|---|---|
| Delete the selected path point (or the last point if none is selected) | `Delete` / `Backspace` |
| Finish editing the path (the next point starts a new path) | `Enter` |
| Delete the selected point of a point gradient | `Delete` / `Backspace` |
| Hide / show fill projection and gradient box handles | `Q` (Paint and Edit modes) |

## Selection

| Action | Key |
|---|---|
| Select All / Deselect / Invert Selection | `Ctrl+A` / `Ctrl+D` / `Ctrl+Shift+I` |
| Add / subtract / intersect selection | `Shift` / `Ctrl` / `Shift+Ctrl` + drag with a selection tool (also on the options bar; with the Selection Pen, `Shift` gives the pen and `Ctrl` gives the eraser) |
| Fixed ratio / from center | `Shift` / `Alt` pressed after starting to drag |
| Quick Mask | `Shift+Q` |
| Erase Selection | `Delete` (Paint mode. In Edit mode, `Delete` deletes the selected object) |
| Copy selection to a new layer | `Ctrl+J` (without a selection, `Ctrl+J` duplicates the layer) |
| Close a polygon / remove its last point | `Enter` (or click the first point or double-click) / `Backspace` |
| Apply / cancel a transform | `Enter` / `Esc` |
| Move by 1 pixel / 10 pixels in Move / Transform | Arrow keys / `Shift` + arrow keys |
| Copy / Cut / Copy Merged / Paste | `Ctrl+C` / `Ctrl+X` / `Ctrl+Shift+C` / `Ctrl+V` (`Ctrl+X` and `Ctrl+V` in Paint mode. The Edit menu works in every mode) |

## Layers

| Action | Key |
|---|---|
| New Layer | `Ctrl+Shift+N` |
| Select layers (toggle / range / add range) | `Ctrl`-click / `Shift`-click / `Ctrl+Shift`-click in the layer list |
| Merge Down / Merge Visible | `Ctrl+E` / `Ctrl+Shift+E` (Paint mode. The Layer menu works in every mode) |
| Duplicate | `Ctrl+J` |
| Group / Ungroup | `Ctrl+G` / `Ctrl+Shift+G` |

## Files and editing

| Action | Key |
|---|---|
| New / Open | `Ctrl+N` / `Ctrl+O` |
| Save / Save As | `Ctrl+S` / `Ctrl+Shift+S` |
| Undo / Redo | `Ctrl+Z` / `Ctrl+Shift+Z` or `Ctrl+Y` |
| Settings | `Ctrl+,` |
| Quit | `Ctrl+Q` |
| Pick Screen Color / Hide Window and Pick Screen Color (Windows only) | `Ctrl+Alt+I` / `Ctrl+Alt+Shift+I` |

## View controls and `Alt`

View controls are the same in 2D and 3D. `Alt` is a view control (rotating the view in 2D, snap orbit in 3D) if it is held at the moment you press the button; if you press it after pressing the button, it is the tool's modifier (From center for shapes, rectangles and ellipses, breaking a path handle, and so on). A drag of a selection or shape that starts with `Alt` held becomes a view rotation.

These combinations no longer work. In 3D, `Shift` + right-drag and `Alt` + `Shift` + left-drag (move; with `Shift` held, a right-drag orbits and an `Alt` + left-drag snap-orbits). In 2D, `R` + left-drag and `Shift` + middle-drag (rotate; with `Shift` held, a middle-drag pans). In 2D, `Alt`-click as a temporary eyedropper (`Alt` + left-drag now rotates the view, and the eyedropper moved to the right button). In 3D, `Alt` + left-drag as a free orbit (it is now the snap orbit).

## The 2D view

| Action | Input |
|---|---|
| Zoom | Wheel, or `Ctrl+Space` + left-drag horizontally (the pressed point is the center; releasing without moving zooms in, and adding `Alt` zooms out) |
| Zoom In / Zoom Out | `Ctrl++` (or `Ctrl+=`) / `Ctrl+-` |
| Pan | Middle-drag, or `Space` + left-drag |
| Rotate | `Alt` + left-drag (15° steps; free if `Shift` is also held. Released without moving with the Clone brush, it sets the clone source) |
| Rotate View Left / Right | `-` / `^` (the `=` key also works) |
| Reset rotation / flip horizontally | `Shift+R` / `H` (in Edit mode `H` hides the selected object's marker, so flip with View → Flip View) |
| Pick a color | Press the right button (move while holding it, and the color where you release is picked) |
| Fit to the view | `Ctrl+0` |

## The 3D view

| Action | Input |
|---|---|
| Orbit | Right-drag |
| Snap orbit | `Alt` + left-drag (when the direction comes within 15° of an axis view (front, back, right, left, top, bottom), it snaps to that view) |
| Move the view forward / back, left / right, down / up | `W` / `S`, `A` / `D`, `Q` / `E` while the right button is held (`Shift` for faster; while it is held, these keys are not used for switching tools and so on) |
| Pan | Middle-drag, or `Space` + left-drag |
| Zoom | Wheel, or `Ctrl+Space` + left-drag (as in 2D) |
| Pick a color | Release the right button without moving it |
| Frame the selected texture set | `.` |
| Cancel an operation | `Esc` |

## Stencil

| Action | Input |
|---|---|
| Rotate / move / resize | Hold `Y` and left-drag (`Shift` snaps to 15°) / middle-drag or `Ctrl` + left-drag / right-drag or `Alt` + left-drag (in 2D and 3D) |
| Bypass the stencil | Hold `N` |
| Cancel an operation | `Esc` |

## Pen

The side buttons act as the right button (in 2D they pick a color; in 3D they orbit, and releasing without moving picks a color; with Polygon Fill in 2D they do nothing). `Alt`, `Space` and `Ctrl+Space` act the same as with the left mouse button. Only a pen tip with neither `Ctrl` nor a side button pressed (and the eraser end) paints, and a pen tip with `Shift` pressed draws a straight line (in 2D and 3D).

## If you get lost

- If you lose the whole model, use View → Frame the Model in the 3D View.
- To put the panels back, use Window → Reset Panel Layout.
- You can also open a `.ylp` by dragging it onto the window.
