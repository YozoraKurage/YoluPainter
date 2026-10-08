# Keyboard Shortcuts

[日本語](../GUIDE_KEYS.md)

A list of keyboard, mouse and pen controls. The app lists the same assignments under Help → Keyboard Shortcuts. On Mac, read `Ctrl` in these tables as `Command` (menus still display `Ctrl`).

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
| Draw a straight line from the end of the previous stroke | `Shift`-click with the brush or eraser (hold `Shift` and drag to lock the direction to 45° steps) |
| Temporarily act as the eyedropper (2D) | `Alt`-click with Brush, Eraser, Fill or Polygon Fill |
| Shape: 45° steps, square or circle / draw from the center | `Shift` / `Alt` |
| Turn Snap to Ruler on or off | `Ctrl+1` |
| Cancel the current stroke, shape or ruler drag, selection operation or path point drag | `Esc`. If there is nothing to cancel, it deselects the selected point, and if there is no point, it deselects the selection. An input field, menu or window that is using the key takes it first |

## Paths and fill points

| Action | Key |
|---|---|
| Delete the selected path point (or the last point if none is selected) | `Delete` / `Backspace` |
| Finish editing the path (the next point starts a new path) | `Enter` |
| Delete the selected point of a point gradient | `Delete` / `Backspace` |
| Hide / show fill projection and gradient box handles | `Q` |

## Selection

| Action | Key |
|---|---|
| Select All / Deselect / Invert Selection | `Ctrl+A` / `Ctrl+D` / `Ctrl+Shift+I` |
| Add / subtract / intersect selection | `Shift` / `Ctrl` / `Shift+Ctrl` + drag with a selection tool (also on the options bar; with the Selection Pen, `Shift` gives the pen and `Ctrl` gives the eraser) |
| Fixed ratio / from center | `Shift` / `Alt` pressed after starting to drag |
| Quick Mask | `Shift+Q` |
| Erase Selection | `Delete` |
| Copy selection to a new layer | `Ctrl+J` (without a selection, `Ctrl+J` duplicates the layer) |
| Close a polygon / remove its last point | `Enter` (or click the first point or double-click) / `Backspace` |
| Apply / cancel a transform | `Enter` / `Esc` |
| Move by 1 pixel / 10 pixels in Move / Transform | Arrow keys / `Shift` + arrow keys |
| Copy / Cut / Copy Merged / Paste | `Ctrl+C` / `Ctrl+X` / `Ctrl+Shift+C` / `Ctrl+V` |

## Layers

| Action | Key |
|---|---|
| New Layer | `Ctrl+Shift+N` |
| Select layers (toggle / range / add range) | `Ctrl`-click / `Shift`-click / `Ctrl+Shift`-click in the layer list |
| Merge Down / Merge Visible | `Ctrl+E` / `Ctrl+Shift+E` |
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

## The 2D view

| Action | Input |
|---|---|
| Zoom | Wheel, or `Ctrl+Space` + left-drag horizontally (the pressed point is the center; releasing without moving zooms in, and adding `Alt` zooms out) |
| Zoom In / Zoom Out | `Ctrl++` (or `Ctrl+=`) / `Ctrl+-` |
| Pan | Middle-drag, or `Space` + left-drag |
| Rotate | `R` + left-drag, or `Shift` + middle-drag |
| Rotate View Left / Right | `-` / `^` (the `=` key also works) |
| Reset rotation / flip horizontally | `Shift+R` / `H` |
| Fit to the view | `Ctrl+0` |

## The 3D view

| Action | Input |
|---|---|
| Orbit | Right-drag, or `Alt` + left-drag |
| Pan | Middle-drag, `Space` + left-drag, or add `Shift` to an orbit |
| Zoom | Wheel, or `Ctrl+Space` + left-drag (as in 2D) |
| Frame the selected texture set | `.` |
| Cancel an operation | `Esc` |

## Stencil

| Action | Input |
|---|---|
| Rotate / move / resize | Hold `Y` and left-drag (`Shift` snaps to 15°) / middle-drag or `Ctrl` + left-drag / right-drag or `Alt` + left-drag (in 2D and 3D) |
| Bypass the stencil | Hold `N` |
| Cancel an operation | `Esc` |

## Pen

The side buttons act as the right button. `Alt`, `Space`, `Ctrl+Space` and `R` act the same as with the left mouse button. Only a pen tip with neither `Ctrl` nor a side button pressed (and the eraser end) paints, and a pen tip with `Shift` pressed draws a straight line.

## If you get lost

- If you lose the whole model, use View → Frame the Model in the 3D View.
- To put the panels back, use Window → Reset Panel Layout.
- You can also open a `.ylp` by dragging it onto the window.
