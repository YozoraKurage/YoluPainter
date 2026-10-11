# Fill Layers and Effects

[日本語](../GUIDE_FILL.md)

This page covers fill layers (layers that paint surfaces with values, images or gradients), effects that sit on a layer's pixels or mask, assets and the library for reusing materials, and recording and playing back actions.

## Create a fill layer

Choose a type from "New Fill Layer" in the Layers panel (the same item is in the Layer menu). It is made right above the selected layer (inside the group, if a layer in a group is selected).

| Type | What it creates |
|---|---|
| Solid Color | A layer painted with a value per channel |
| Gradient Decal | A layer that paints values over a range in the 3D view (Box, Sphere, Plane (Linear)) |
| Image | A layer painted with an asset image. "Import from File…" in the list can bring in a PNG |
| Decal | A layer that places an asset image on the model like a sticker |

The "Fill" section in Properties holds the value for each channel. The button next to a channel adds its value from the paint color, and the delete button removes it. Color channels (Color, Emission) are set by clicking the swatch to open the color window; scalar channels (Roughness, Metallic, Height) use a 0–1 slider.

### Paint several channels at once

Turn on "Paint several channels at once" in "Paint Channels" at the end of Tool Properties (for the brush, eraser, Fill, Polygon Fill, Gradient, Shape, Path and Eyedropper tools, and for the selection tools Rectangle Select, Ellipse Select, Lasso, Polygon Select, Magic Wand and Selection Pen; it starts closed. With the selection tools, filling or erasing a selection uses this set), and one stroke paints every checked channel with its own value (Color uses the paint color, Emission a color, Roughness, Metallic and Height 0–1, and Normal a tilt). It works with 2D, 3D, Fill, Polygon Fill and paths, and one undo takes all channels back. When off, the selected channel is painted with the paint color.

## Paint with an image

The "Image" section sets the image the painting channel reads. Drag one from the Assets panel onto the field, or click the field and pick from the list. "Read as" says whether the image values are color (sRGB) or data (linear), and applies to every layer that reads the image. "Anisotropic filtering" is on from the start and reduces blur on images projected at an angle or tiled unevenly.

### Projection and placement

The "Projection" section chooses how the image is applied to the model.

- UV: lays the image on the UV square.
- Tri-planar: projects along three axes of a box and mixes them where the surface turns ("Blend"). No UV seams show.
- Planar, Spherical, Cylindrical: project through the box's front face, or around the box's center.
- Decal: projects through the box's front face, only inside the box, cut out by the image's alpha. "Depth edge", "Back faces" and "Face edge" soften how it fades.

"Tiling", "Offset", "Rotation" and "Outside" (Repeat, Clamp to edge, Transparent) adjust the mapping. Projections other than UV read the baked Position and World Normal mesh maps (baking is in [GUIDE_3D.md](GUIDE_3D.md)). While a map is missing, the reason is shown in the field and the value appears instead of the image. A set opened from a `.ylp` that lacks a needed map or image is read-only until they are available.

Except for UV, "Handles in 3D View" (also `Q`) moves the box. Arrows and the center square move it, rings rotate it (`Ctrl` for 15° steps), and face knobs resize it (`Shift` for both sides), all in one gizmo. "Center", "Rotation" and "Size" can also be typed, and "Fit to the model (its bounds)" fits the box to the model.

Dropping an asset image onto the model in the 3D view adds a decal with the image's aspect ratio, facing the surface under the cursor, on the current painting channel in one undo step (the top of the view is the top of the image). It cannot be placed on a face of another texture set.

## Paint with a gradient

Both can be placed on any channel except Normal.

### Gradient Decal

In the "Gradient Decal" section, "Edit in 3D View" shows a shape (Box, Sphere, Plane (Linear)) with handles to set the range. The values inside the range are painted, following positions on the model, so no UV seams show. It needs the Position mesh map.

"Falloff" (not for Plane) sets how the edge fades, and "Invert" reverses the gradient. The color and opacity stops, the gradient presets and the value curve are the same ramp field as the gradient map ([GRADIENT_MAP.md](../GRADIENT_MAP.md), in Japanese).

### Point Gradient

In the "Point Gradient" section, you place points and each point's color and opacity are blended smoothly by distance. "Model" blends by position on the model's surface (it needs the Position mesh map and does not break at UV seams); "UV" blends by position on the texture's UV.

While "Edit Points" is on, click a surface in the 3D view or the 2D canvas to add a point, drag a marker to move it, and press `Delete` to remove the selected point (the last one cannot be removed). Change a point's color from its swatch in the list, and set the flatness around points with "Spread". There can be up to 64 points, and they cannot share a channel with an image or a gradient decal.

## Layer effects

Effects are filters (which change pixels) and generators (which make values) placed on a layer's pixels or mask. They apply to the standard channels and are listed under the layer's row, indented (the higher a row, the later it applies).

### Add an effect

"Add Filter" and "Add Generator" are separate entrances.

- The two buttons at the bottom of the Layers panel
- The Layer and Layer Mask sections of Properties (the mask section says "Add Filter to Mask" and "Add Generator to Mask")
- The layer context menu, and "Add Filter ▸" and "Add Generator ▸" in the Layer menu
- The Filter menu (filters and anchors only)

Thumbnails choose whether an effect goes on the layer or on its mask. Click the layer thumbnail to target the layer's pixels, or the mask thumbnail to target the mask (the targeted thumbnail gets a blue frame); the add buttons and Properties then follow the target. Clicking a row of a mask effect also targets the mask. Right-clicking the mask thumbnail offers adding to the mask, Invert, Enabled and Delete Mask.

The eye on a row turns the effect on or off, and the up, down and delete buttons reorder or remove it (the same items are in the context menu). Click a row to see its settings in Properties. Every effect has "Strength", and a generator's "Combine" chooses how it meets the values below (Multiply, Replace, Screen, Max, Min, Add, Subtract).

One operation is one undo step, and a slider drag counts as one. You cannot add to a layer locked with "All" (including through a parent group).

### Kinds of effects

There are 23 filters, listed in the menu as blurs, edges and shapes, noise, value adjustments and color corrections. Besides Gaussian Blur, Sharpen (Unsharp Mask), Noise, Levels, Invert, Normalize (Layer) and the color corrections that share the adjustment layer fields, these 10 are available:

| Name | What it does |
|---|---|
| Histogram Scan | Pushes values above the position toward 1 and below toward 0; contrast sharpens the cut (scalar channels and masks only) |
| Histogram Range | Compresses values into a range around the position (scalar channels and masks only) |
| Slope Blur | Smears values along the slopes of a built-in noise |
| Directional Blur | Blurs along a line in both directions of the angle |
| Warp | Distorts the image by shifting where it is read along the slopes of a built-in noise |
| Dilate / Erode | Grows bright areas with the maximum of a round window, or shrinks them with the minimum (scalar channels and masks only) |
| Edge Detect | Blurs, then extracts the edge strength (scalar channels and masks only) |
| High Pass | Keeps the difference from a blurred copy around 0.5 gray |
| Median | Removes specks with the median of a square window |
| Glow | Adds a blurred copy of the parts brighter than the threshold (color channels only) |

There are 15 generators, in these groups:

- Values from baked mesh maps: Edge Wear, Dirt, Position Gradient, Shape Gradient, Thickness, Direction, Light (light and shade from the baked normals and a light direction), Mask Builder (combines the baked curvature, AO (ambient occlusion), height (position) and thickness with weights and levels), ID Color
- Values from the model's UV islands: UV Island Variation (one value per island)
- Work without maps: Noise, Grunge, Pattern (stripes, checker, dots, border or grid in UV space), Image (reads an asset image with the same projections as a fill layer). Noise and Grunge are made on the model when the position map exists, and in UV space otherwise
- Anchor: reads the result of a layer below

A generator whose map is missing, stale or cannot be matched to the model shows a mark and the reason on its row and passes its input through (bake to make it work). For ID Color, press "Pick from ID Map", then click the 2D canvas or the 3D view to add the color there (`Ctrl`+click removes it, up to 32 colors; press the button again to finish). An anchor is a named marker you put on a layer or mask, which Anchor generators on layers above can read.

### Read across UV seams

Filters that read neighboring pixels, such as blur and sharpen, also read the pixels across the edges of UV islands (the neighboring faces in 3D) when there is a model. "Across UV seams" in the effect fields toggles it for every filter in that texture set.

## Save assets for reuse

The Assets panel (top right) has two places. "Project" holds this project's assets, which are saved in the `.ylp` (up to 256). "Library" is your own folder (set by "Library folder" in the settings), shared by all your projects.

Items are Images, Brushes, Materials, Smart Materials and Smart Masks; filter them with the icons on top and search by name. Bundled smart materials that can be placed right away are listed too.

- "Save Layer": puts the selected layer (with its contents, for a group) into the project's assets as a smart material. "Save Mask" makes the mask a smart mask. The save runs in another thread, with "Saving" and "Cancel" in the panel; meanwhile deleting from or importing into the assets is refused with "Already saving". Layers that use user channels, noise, grunge, color corrections or the 0.5.0 effects cannot be written to `.ylsmart`, so they are refused with a reason.
- "Place" (also double-click, right-click, or dragging onto the Layers list): puts the item above the selected layer as new layers. In a texture set of a different size it is stretched to fit, and one undo takes it back. A smart mask uses "Apply to Mask" and replaces the selected layer's mask. An item that cannot be placed just sits in the list with the reason.
- "Put into the library" and "Use in this project": move items between the assets and the library.
- `.ylsmart` can be imported and exported (the Unity version up to 0.4.x reads it too; the Unity bridge does not import it from 0.5.0).

"Save as Material" in a fill layer's context menu writes the fill values, gradients, projection and filters to `Materials` in the library as a `.ylmaterial` (named after the layer; a number is added when the name is taken). Masks, clipping, opacity, blend mode, visibility and locks are not kept, and a placed material becomes the defaults of a new fill layer. A fill that uses an asset image cannot be saved (the item is disabled and the tooltip gives the reason).

## Record and play back actions

The Actions panel (Window → Actions) records layer, mask and effect operations (add, delete, reorder, visibility, opacity, blend mode, clipping, locks, channel enable, fill values, adjustment and effect values) and plays them back on another texture set or document.

1. Press "Record" and operate on layers and effects. A mark appears in the panel when an operation cannot be recorded, such as painting, selections, transforms, merging or duplicating. Exports and saves are not recorded.
2. Press "Stop Recording" and the action is added to the list. Double-click to rename it, and drag to reorder.
3. Select an action and press "Play" to apply it to the current texture set. The whole run is one undo step.

The layer that was selected when recording started, and the layers and effects created during recording, point to the same place in another document. Other layers are referred to by name, and an operation on a name shared by several layers is not recorded. Actions live in `actions/` in the settings folder and are not stored in the `.ylp`. They can also be applied with the command-line `run-action` ([CLI.md](CLI.md)) and the MCP tool `action_run` ([MCP.md](MCP.md)).

## Handing files to older versions and Unity

A `.ylp` that uses point gradients, an image with anisotropic filtering turned off, or a 0.5.0 effect cannot be opened by the 0.4.x standalone app or the Unity version up to 0.4.x. See [GUIDE_FILES.md](GUIDE_FILES.md) for what to watch when handing files over.
