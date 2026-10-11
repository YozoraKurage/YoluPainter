# Layers and Channels

[日本語](../GUIDE_LAYERS.md)

This page covers creating and arranging layers, masks, adjustment layers and channels. For fill layers and the effects (filters and generators) you add to layers, see [GUIDE_FILL.md](GUIDE_FILL.md).

## Creating and arranging layers

Create layers with the buttons below the Layers panel or the Layer menu in the menu bar. The same items are in the right-click menu of a layer and of the empty area of the list (the buttons below the panel are only New Layer, New Fill Layer and New Adjustment Layer). A new layer is made right above the selected layer (inside the group, if a layer in a group is selected), and on top when no layer is selected.

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

Group Layers (`Ctrl+G`) puts the selected layers in a group. Groups can be nested and are opened and closed with the arrow in the row. Ungroup is `Ctrl+Shift+G`. With a group's Pass through on, its contents are blended with the layers below as if not grouped. With it off, they are composited together first.

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

The top row of the Layers panel sets the selected layer's blend mode and opacity (for the painting channel). There are 26 blend modes, and groups also have Pass through. Clip to the Layer Below (Clipping in the right-click menu) restricts the layer to the shape of the layer below it.

Each channel can have different values too. Choose “{channel} only” at the top of the blend mode menu to give that channel its own blend mode and opacity. Which channels a layer is used in is set in Channels in Properties (what the layer holds in a channel is kept when you turn it off).

Reference Layer in the right-click menu marks the layers the Fill tool's Similar colors reads ([GUIDE_PAINT.md](GUIDE_PAINT.md)).

## Using masks

A mask hides part of a layer. It is shared by all channels.

1. Select a layer and press Add Layer Mask (the button below the list, the right-click menu, or Properties).
2. Click the mask thumbnail in the layer's row to paint on the mask, and click the pixel thumbnail to go back to the pixels. Paint on Mask in the right-click menu also switches.
3. Painting hides, and erasing reveals.

In Properties you can change the mask's Density (0% is the same as no mask), Enabled (off is the same as no mask, and the mask is kept) and Invert. While you paint on the mask, the same fields (the Layer Mask section) also appear at the end of Tool Properties for the painting tools (brush, eraser, Fill, Polygon Fill, Gradient, Shape, Path and Eyedropper) and the selection tools (Rectangle Select, Ellipse Select, Lasso, Polygon Select, Magic Wand and Selection Pen). To make a mask from a selection, use Make the Selection a Layer Mask ([GUIDE_SELECT.md](GUIDE_SELECT.md)). Delete Layer Mask removes it.

## Adjustment layers

Choose from these 9 in New Adjustment Layer. They adjust the look of the layers below without rewriting their pixels. Set the values in Properties.

Invert, Levels, Hue / Saturation, Gradient Map, Tone Curve, Color Balance, Brightness / Contrast, Threshold and Posterize.

For the Gradient Map ramp, see [GRADIENT_MAP.md](../GRADIENT_MAP.md) (Japanese).

## Working with channels

A layer holds pixels for each channel. The six standard channels are Color, Roughness, Metallic, Height, Normal and Emission. In the Channels panel you choose what to paint on and what to show.

- Click a row (or the brush icon in it) to make that channel the one you paint. The eye icon shows that channel on the 2D Canvas. The right-click menu also has Paint This Channel and Show in Canvas.
- Add Channel creates your own channel (a user channel). Double-click its name to rename it, and choose its type at the right end of the row (the format `sRGB8`, `L8` or `RGB8`). Delete Channel works only on user channels.

### Normal settings

The Normal section of the Channels panel holds document settings. They are saved in the `.ylp` and changes can be undone. The same values apply to the Normal in the 3D view and in exports.

| Item | What it does |
|---|---|
| Height → Normal | When on, adds the normal derived from the Height channel under the painted Normal layers. It is regenerated from Height each time and never painted into a layer |
| Strength | Texels of rise for the full height range (0 → 1). A negative value turns bumps into dents |
| Edges | Clamp uses the edge texel for the slope at the canvas edge, and Wrap (tiling) reads the opposite edge (for tiling textures) |
| File Y | The green direction of the images written by exports: OpenGL (Y+) or DirectX (Y−). Unity uses OpenGL, and the `.ylp` texture and the preview always do |
