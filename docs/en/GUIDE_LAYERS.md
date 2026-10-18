# Layers and Channels

[日本語](../GUIDE_LAYERS.md)

This page covers creating and arranging layers, masks, adjustment layers and channels. For fill layers and the effects (filters and generators) you add to layers, see [GUIDE_FILL.md](GUIDE_FILL.md).

## Creating and arranging layers

Create layers with the buttons in the action strip of the Layers panel (right below the blend mode and opacity row) or the Layer menu in the menu bar. The same items are in the right-click menu of a layer and of the empty area of the list. The strip has six buttons: Clipping at the left end, and New Layer, New Fill Layer, Group Layers, New Adjustment Layer and Delete Layer to the right (adding filters and generators, layer masks, and Move Layer Up and Down are in the right-click menu). A new layer is made right above the selected layer (inside the group, if a layer in a group is selected), and on top when no layer is selected. Only a Solid Color fill layer is made right below the selected layer.

| Item | What it creates |
|---|---|
| New Layer (`Ctrl+Shift+N`) | A layer that holds pixels |
| New Fill Layer | Solid Color, Gradient Decal, Image or Decal ([GUIDE_FILL.md](GUIDE_FILL.md)) |
| New Adjustment Layer | See “Adjustment layers” below |
| New Path Layer | A layer with one path that has no points. It switches to the Path tool, and the first point you place becomes a path of this layer ([GUIDE_PATHS.md](GUIDE_PATHS.md)) |
| New Group | A container for layers |

- **Select**: click a row. `Ctrl`+click adds or removes, `Shift`+click selects a range, and `Ctrl+Shift`+click adds a range.
- **Rename**: double-click the name in a row, or choose Rename from the right-click menu.
- **Reorder**: drag a row. If you press a selected row and drag, you carry all the selected rows. Move Layer Up and Move Layer Down also work.
- **Visibility**: toggle with the eye icon in a row.
- **Rulers**: the row of a layer (or group) that has rulers shows a Ruler icon to the right of its thumbnail. How to click, `Shift`+click, drag and right-click it is in “Ruler” in [GUIDE_PAINT.md](GUIDE_PAINT.md).
- **Delete and duplicate**: Delete Layer, and Duplicate in the right-click menu (`Ctrl+J`). Undo brings them back.

When several layers are selected, you can delete, duplicate, group, merge, toggle visibility, move up or down by one position, drag and lock them all together. Painting still targets a single layer.

### Groups

Group Layers (`Ctrl+G`) puts the selected layers in a group. Groups can be nested and are opened and closed with the arrow in the row. Ungroup is `Ctrl+Shift+G`. With a group's Pass through on, its contents are blended with the layers below as if not grouped. With it off, they are composited together first. If even one layer right above the group is clipped to it (even an invisible one), a pass-through group is also composited together first.

### Merging

| Action | Key |
|---|---|
| Merge Down (for a group, Merge Group; with several layers selected, Merge Layers) | `Ctrl+E` |
| Merge Visible | `Ctrl+Shift+E` |

A single undo restores the original layers. If merging would change the appearance by more than one rounding step, the number of affected pixels is shown for confirmation. When merging is unavailable (no layer below, hidden layers, locks and so on), a short reason is shown and nothing changes.

### Locks

There are four locks. Transparent pixels keeps each pixel's transparency when you paint and changes only the color. Image pixels makes the pixels unchangeable (you can still move the layer and paint its mask). Position stops moving and transforming. All stops changes to pixels, position and the layer's settings.

Toggle them through the lock icon in a layer row (click to remove the layer's own lock), the lock items in the right-click menu (Lock Transparent Pixels, Lock Image Pixels, Lock Position and Lock All), or Lock in Properties. A group's locks apply to its contents. Locks are saved and restored in `.ylp`, PSD and `.ylsmart`.

## Compositing

The top row of the Layers panel sets the selected layer's blend mode and opacity (for the painting channel). There are 26 blend modes, and groups also have Pass through. Clip to the Layer Below (the button at the left end of the strip, or Clipping in the right-click menu) restricts the layer to the shape of the layer below it. The row of a clipped layer is marked by a thin vertical strip left of its thumbnail (it is not indented).

Each channel can have different values too. Choose “{channel} only” at the top of the blend mode menu to give that channel its own blend mode and opacity. Which channels a layer is used in is set in Channels in Properties (what the layer holds in a channel is kept when you turn it off).

Reference Layer in the right-click menu marks the layers the Fill tool's Similar colors reads, and the layers whose lines a brush with Do not cross lines of reference layer stays inside ([GUIDE_PAINT.md](GUIDE_PAINT.md)).

## Layer sets

A layer set is a named group of layers inside one texture set. For each channel, you choose which layer set's composite is the final result (what the 3D view, exports and Live Link show): the output assignment. Use it, for example, to paint only some channels such as a shadow mask with separate layers, or to keep two versions and compare them by switching the assignment. A new document has only one layer set ("Material"), and it outputs every channel. While there is just that one layer set outputting every channel, the rows and dropdowns described below are not shown.

- **Create**: use the "+" ("Add Layer Set") in the "Layer Set" column of the panel opened by the paint channel button in the top right of the 2D canvas, or New Layer Set in the right-click menu of a layer set row in the Texture Sets panel. A new layer set starts with a white solid fill and an empty layer (the layer set made when you add a user channel is that channel's output, so it gets no white fill; neither does a set that receives Live Link originals, see [LIVELINK.md](LIVELINK.md)). The right-click menu also has Duplicate, Rename and Delete (the last one cannot be deleted). Each is one undo step.
- **Choose**: in the Texture Sets panel, the layer sets of a set that has two or more are listed indented below its row (double-click to rename, drag to reorder). Clicking one makes it the current layer set, and the list in the Layers panel changes. The dropdown at the top of the Layers panel and the panel in the top right of the canvas choose it too.
- **2D canvas**: the canvas, the Navigator, the eyedropper and the "All layers" of color-based ranges read the composite of the current layer set. You can keep painting while you look at a layer set that is not in the output (the 2D display is composited on the CPU while you do). The 3D view, exports and Live Link always show the final result.
- **Output assignment**: the dropdown in each row of the Channels panel ("Output layer set") chooses the layer set whose composite is that channel's final result, or None (one undo step). The final result of a channel set to None is that channel's default value (white for Color). A layer set that outputs nothing is marked with a broken chain in the Texture Sets panel, and when the painting channel is not output from the current layer set, the same mark appears at the top of the Layers panel. Painting in the 3D view on a channel the current layer set does not output shows nothing in the 3D view.
- **Move layers**: Move to Layer Set in the right-click menu of a layer moves it to the top of another layer set (a group moves with its contents). Reordering, moving into or out of a group, grouping and merging across layer sets are not possible.
- **Delete**: deleting a layer set deletes its layers too, and the channels it output are set to None (they do not move to another layer set).
- **User channels**: creating a user channel also creates a layer set named after it, and that channel's output is assigned to it.

Effects (Anchors, generators, the clone source and so on) read only within the layer set of their layer. The current layer set is not saved, so a reopened document starts from the first layer set. A document with two or more layer sets, or with a channel set to None, cannot be opened by 0.6.x. In PSD, each layer set is exported as its own group ([GUIDE_FILES.md](GUIDE_FILES.md)).

## Using masks

A mask hides part of a layer. It is shared by all channels.

1. Select a layer and choose Add White Layer Mask (a mask that hides nothing) or Add Black Layer Mask (a mask that hides everything) from the right-click menu. White Mask and Black Mask in Properties do the same.
2. Click the mask thumbnail in the layer's row to paint on the mask, and click the pixel thumbnail to go back to the pixels. Paint on Mask in the right-click menu also switches.
3. Painting hides, and erasing reveals.

Drag a mask thumbnail onto another layer's row (a group works too) to move the mask there; if that layer already has a mask, it is replaced. Hold `Alt` when you release to copy instead. Copy Mask and Paste to Mask in the right-click menu copy too (with several layers selected, the mask is pasted to all of them). While you paint on a mask, `Ctrl+V` pastes the clipboard image into the mask (white shows, black hides; a color image is read by its brightness).

In Properties you can change the mask's Density (0% is the same as no mask), Enabled (off is the same as no mask, and the mask is kept) and Invert. While you paint on the mask, the same fields (the Layer Mask section) also appear at the end of Tool Properties for the painting tools (brush, eraser, Fill, Polygon Fill, Gradient, Shape, Path and Eyedropper) and the selection tools (Rectangle Select, Ellipse Select, Lasso, Polygon Select, Magic Wand and Selection Pen). To make a mask from a selection, use Make the Selection a Layer Mask ([GUIDE_SELECT.md](GUIDE_SELECT.md)). Delete Layer Mask removes it.

## Adjustment layers

Choose from these 9 in New Adjustment Layer. They adjust the look of the layers below without rewriting their pixels. Set the values in Properties.

Invert, Levels, Hue / Saturation, Gradient Map, Tone Curve, Color Balance, Brightness / Contrast, Threshold and Posterize.

For the Gradient Map ramp, see [GRADIENT_MAP.md](../GRADIENT_MAP.md) (Japanese).

## Working with channels

A layer holds pixels for each channel. The six standard channels are Color, Roughness, Metallic, Height, Normal and Emission. In the Channels panel you choose what to paint on and what to show.

- Click a row (or the brush icon in it) to make that channel the one you paint. The eye icon shows that channel on the 2D Canvas. The right-click menu also has Paint This Channel and Show in Canvas.
- Add Channel creates your own channel (a user channel; it also creates a layer set named after it, see "Layer sets" above). Double-click its name to rename it, and choose its type at the right end of the row (the format `sRGB8`, `L8` or `RGB8`). Delete Channel works only on user channels.

### Normal settings

The Normal section of the Channels panel holds document settings. They are saved in the `.ylp` and changes can be undone. The same values apply to the Normal in the 3D view and in exports.

| Item | What it does |
|---|---|
| Height → Normal | When on, adds the normal derived from the Height channel under the painted Normal layers. It is regenerated from Height each time and never painted into a layer |
| Strength | Texels of rise for the full height range (0 → 1). A negative value turns bumps into dents |
| Edges | Clamp uses the edge texel for the slope at the canvas edge, and Wrap (tiling) reads the opposite edge (for tiling textures) |
| File Y | The green direction of the images written by exports: OpenGL (Y+, Unity) or DirectX (Y−). Unity uses OpenGL, and the `.ylp` texture and the preview always do |
