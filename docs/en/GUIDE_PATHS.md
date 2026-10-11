# Paths

[日本語](../GUIDE_PATHS.md)

The Path tool (`P`) draws an editable curve through a series of points, on the 2D canvas or on the surface of the model. Every time you change a point or a setting, the pixels are redrawn from the curve. The keys are listed in [GUIDE_KEYS.md](GUIDE_KEYS.md).

## Draw a path

1. Choose the Path tool (`P`) in the left toolbar and select the layer to draw on.
2. Click the canvas, or a surface of the model in the 3D view, to place the first point. If an ordinary layer is selected, a new path layer is created above it. If a path layer or a fill layer is selected, the point becomes a path of that layer.
   To make the layer before placing a point, use New Path Layer in the Layer menu (and the right-click menu). It makes a layer with a path that has no points right above the selected layer and switches to the Path tool, and the first point (in 2D or in 3D) becomes a path of that layer.
3. Keep clicking to add points at the end. Click on the curve to insert a point into that segment, and drag a point to move it.
4. `Delete` (or `Backspace`) removes the selected point, or the last point when none is selected.
5. `Enter` leaves the path. The next click starts a new path on the same layer. `Esc` cancels a drag; otherwise it clears the point selection; when no point is selected, it leaves the path.

With three or more points, "Close" in the tool bar turns the path into a loop (the first and last points sit at the same place), and "Open" undoes it. Dragging with `Shift` held selects the points inside the rectangle, and width, corner, handles and delete then apply to all of them.

Each point operation is one undo step. Dragging a point redraws the path on release, and losing window focus commits what you have done so far. Points cannot be placed outside the canvas. You cannot paint on a path layer by hand, and locked layers and painting in progress refuse edits.

## Change the width of a point

"Width" in the tool bar is the width of the selected point (it stands in for pen pressure). Different widths along the path change its thickness smoothly. Turn on "Width changes the size", "Width changes the opacity" or "Width changes the flow" in the path brush to apply the width to the drawn size and density.

## Shape the curve with corners and handles

At first, a point joins its neighbors smoothly. With a point selected you can:

- "Corner": break the curve at the point. Double-clicking the point also toggles it.
- "Handles": show handles on the point and drag them to set the curvature. Showing them does not change the shape. When you drag a handle, the opposite handle follows to keep the curve smooth. Press `Alt` after you start dragging to move only the handle you drag (if `Alt` is already held when you press, it is a view control); hold `Ctrl` to scale both handles by the same ratio.
- Press the same button again to return to a smooth point.

## Put several paths on one layer

"Path" in the tool properties lists the paths of the layer, with the top row drawn last. Click a row to edit that path; double-click to rename it. A path hidden with the eye icon is not drawn into the pixels, but its points and settings stay.

The icons below the list are New Path (`Enter`), Duplicate Path, Move Up, Move Down and Delete Path. The context menu has Rename, Visible, Copy, Paste, Duplicate, Paste Settings, Paste Positions, Move Up, Move Down and Delete Path. A copied path can be pasted into another layer, and pasting into a layer without paths creates a new path layer. "Paste Settings" copies only the brush and the set of channels to paint; "Paste Positions" copies only the point positions.

A layer holds up to 256 paths, either all in 2D or all in 3D. Later paths lie on top, and an Erase path erases the pixels of the paths before it.

## What a path draws

"Type" in "Path Brush" chooses what is drawn along the curve.

| Type | What it draws |
|---|---|
| Stroke | A round or image tip placed along the curve |
| Ribbon | An asset image laid along the curve. "Layout" is either "Tile" (the whole image once per step; the gap is "Tile Spacing") or "Stretch" (one image over the whole path). It cannot be chosen while the assets contain no image |
| Fill | The inside of a closed curve. An open path is closed from its end to its start before filling |
| Smudge | Drags the earlier pixels along the curve. "Strength" can be changed |
| Erase | Erases along the curve |

Choose the "Tip" from Round, the built-in tips or the current brush's tip. An image tip also offers "Angle" and "Follow Path", which turns the tip along the direction of the curve. Size ("Width" for a ribbon), Hardness, Anti-aliasing, Spacing, Opacity and Flow belong to each path.

"Use Brush" redraws the path with the current brush and the set of channels chosen in "Paint Channels" in Tool Properties. With a set chosen, one path paints several channels at once ([GUIDE_FILL.md](GUIDE_FILL.md)). "Presets" saves the type, brush, set of channels, tip, depth and symmetry setting under a name (in `path_presets/` in the settings folder).

## Paths on the model

The same operations work in the 3D view. Points are bound to the model's triangles and saved in the `.ylp`. Points cannot be placed on faces of another texture set or off the model.

- Paths are drawn on the unposed shape (the FBX rest pose), so changing the pose does not change the thickness and spacing on the UV.
- The depth searched along the normal starts out from the point spacing and the brush size. Turn off "Automatic projection depth" to set "Projection Depth" yourself. With a shallow depth, parts away from the surface are not drawn.
- A Fill path can be drawn only when all its points are inside one UV island.
- A path drawn on another model shows "Drawn on another model" and cannot be edited.
- When you replace the model, paths are rebound to nearby faces of the same material and redrawn. A path that cannot be rebound keeps its current pixels and loses the path (only paths on layers locked with "All" stay bound to the previous model). The result reports the counts and the first reason.

## Symmetry and direction

Turn on "Symmetry" to draw the mirrored path as well. When you turn it on, the path copies the values of the symmetry ruler in effect for its layer (a 2D path uses a 2D symmetry ruler and a 3D path a 3D symmetry ruler; without a symmetry ruler in effect it does not turn on). Both line symmetry (diagonal, and 6 or more lines too) and rotational symmetry can be copied. A 3D path can copy only a symmetry ruler with 2 lines in line symmetry (a single mirror plane), and cannot turn it on while the model is posed. The path keeps its own values, so moving the ruler later does not change a path already drawn. The mirrored side has no points of its own; it is made when drawing. "Reverse" reverses the order of the points, which changes the direction of tips and ribbons.

## Add a path to a fill layer

With a fill layer selected, "Add Path" in the "Paths" section of Properties starts a new path with the Path tool. The path lies over the fill and its effects, and effects do not apply to it. A path on a fill layer cannot be turned into pixels.

## Turn a path into pixels

"Rasterize" in the tool bar (also "Rasterize Path" in the layer context menu) keeps the current pixels and removes the path, so the layer becomes an ordinary layer.

## Handing files to older versions and Unity

A `.ylp` that uses two or more paths, path types, tips, symmetry, corners or handles, or paths on fill layers cannot be opened by the 0.4.x standalone app or the Unity version up to 0.4.x. See [GUIDE_FILES.md](GUIDE_FILES.md) for what to watch when handing files over.
