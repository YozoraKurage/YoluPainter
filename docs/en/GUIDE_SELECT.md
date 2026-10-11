# Selection and Transform

[日本語](../GUIDE_SELECT.md)

This page covers making and using selections, remembering them, moving and transforming layers, and copy and paste. Selections can be made on the 2D Canvas and in the 3D View. The current selection is saved in the `.ylp` for each texture set.

## Making a selection

| Tool | Key | How to use |
|---|---|---|
| Rectangle Select, Ellipse Select | `M` / `Shift+M` | Drag. A rectangle has a Corner radius |
| Lasso | `L` | Drag around the area |
| Polygon Select | `Shift+L` | Click to place points. Close it with `Enter` (or by clicking the first point or double-clicking), and remove the last point with `Backspace` |
| Magic Wand | `W` | Selects colors close to the pixel you press. It has Tolerance, Contiguous and Sample All Layers. On the 2D Canvas, when a symmetry ruler is in effect and Snap to Symmetry Ruler (on at first) is on, it selects from every copy of the pressed point and combines their union with the selection by the current creation mode as one selection (one undo step; copies outside the canvas are not used, and copies that land on the same pixel count once). If you press outside the canvas, it counts as pressing the edge pixel and selects its copies. The 3D View does not use the copies (this switch cannot be pressed while the 2D Canvas is not shown) |
| Selection Pen, Selection Eraser | `S` | Paint like a brush to add or remove selection amount. Size, hardness and opacity are shared with the brush |
| ID Color Select | `Shift+W` | Selects from the colors of a baked ID map |

In the 3D View, Rectangle Select, Ellipse Select, Lasso, Polygon Select, Magic Wand and ID Color Select work. You draw the shape on the screen of the 3D View, it is projected onto the texture pixels of the visible faces, and it becomes the same selection as on the 2D Canvas ("Select by dragging on the screen" in [GUIDE_3D.md](GUIDE_3D.md)). The Selection Pen and Selection Eraser also work in the 3D View: they paint the texture pixels of the face you press to add to or remove from the selection (while Quick Mask is on, the brush and the eraser do the same). The selection's edge is shown on the 2D Canvas and on the visible faces in the 3D View.

The list in the Tools panel shows these selection tools, and clicking a row switches to that tool. Choose how to combine with the existing selection with New, Add, Subtract and Intersect on the options bar. The same row is at the top of Tool Properties, where it shows only New, Add and Subtract and a ⋯ button (All modes) at first; pressing ⋯ also shows Intersect (while Intersect is chosen, all four stay shown). For the Selection Pen the row has two buttons, Pen and Eraser. Hold `Shift` to add, `Ctrl` to subtract, and `Shift+Ctrl` to intersect. With the Selection Pen, `Shift` gives the pen and `Ctrl` gives the eraser. For rectangles and ellipses, pressing `Shift` after you start dragging holds the aspect ratio, and holding `Alt` grows the shape around the pressed point. You can also turn on Fixed ratio and From center. Anti-alias sets whether the edge is smooth.

The Select menu has these too:

| Action | Key |
|---|---|
| Select All / Deselect / Invert Selection | `Ctrl+A` / `Ctrl+D` / `Ctrl+Shift+I` |
| Quick Mask | `Shift+Q` |

`Esc` cancels an operation in progress, and with nothing in progress it deselects.

### Selecting by ID color

ID Color Select picks the area whose baked ID-map color is close to the pressed pixel (in the 3D View, the UV position of the face you press). Tolerance sets how close. You can also give each mesh part a color by hand. Manual ID colors are saved in the `.ylp` and come back when you reopen it. If you open a model other than the one the colors were set for, they are kept as another model's colors, and editing them and baking the ID map are refused. For baking the ID map, see [GUIDE_3D.md](GUIDE_3D.md).

## Refining and using a selection

- **Refine**: Grow, Shrink, Border, Feather and Sharpen Edge in the Select menu change the shape. An item ending in `…` opens a window for Radius and Edge lock. With Edge lock on, the selection is treated as continuing past the canvas edge, so Shrink, Border and Feather do not pull away from it.
- **Quick Mask**: shows the selection as a red overlay, and you can fix it with the brush and the eraser. The brush and the eraser in the 3D view fix it too, on the texture pixels of the faces you paint, and the red overlay is shown on the faces in the 3D view too ([GUIDE_3D.md](GUIDE_3D.md)).
- **Painting only inside a selection**: the brush and the eraser affect only the inside. The Fill tool and a shape's Fill do too.
- **The button bar**: after you make a selection, a bar appears below it with Deselect, Invert, Grow, Shrink, Feather, Fill with the Paint Color, Erase Selection (`Delete`), Copy to a New Layer (`Ctrl+J`), Make the Selection a Layer Mask and Remember Selection…. Drag the handle at its left end to move it. Turn it off with Show the Selection Button Bar in the Select menu. Without a selection, `Ctrl+J` duplicates the layer.

## Remembering selections

Select → Remembered Selections… (or Remember Selection… on the button bar) opens the Remembered Selections window. Give the current selection a name and press Remember to keep it (up to 32 per texture set). A remembered selection can be recalled with New, Add, Subtract or Intersect, renamed and removed. Each is one undo step. When you change the size of a texture set, remembered selections are rebuilt the same way as the current selection, and any that nothing remains of after shrinking are dropped and reported.

A `.ylp` that uses remembered selections cannot be opened by older apps (the 0.3.x standalone and the Unity version), which refuse it and say why ([UNITY.md](UNITY.md)).

## Moving and transforming

Move / Transform (`V`) moves the selected layer. For a group it moves the contents, with several layers selected it moves them all, and with a selection it moves the pixels inside and the selection itself.

- While you drag, only the transformed outline is shown. Release the pointer or press `Enter` to apply. `Esc`, losing the window's focus or switching tools cancels without changing anything. Each operation is one undo step.
- Use the bounding-box handles to move, scale (`Shift` keeps the aspect ratio) and rotate (`Shift` snaps to 15°). The arrow keys move by 1 pixel (`Shift`: 10).
- Flip Horizontal, Flip Vertical and Rotate 90° Clockwise and Counter-clockwise are in the Edit menu and on the options bar.
- In the Tool Properties panel, Numeric lets you set position (X, Y), scale (Width, Height) and Angle by number and press Apply (Reset returns to the starting values). Resampling is Bilinear (smooth) or Nearest (keeps hard pixels), and it is also on the options bar.

Choose the kind of transform in the sub tool list. Normal is the one above. Free and Perspective change the shape by moving corners, and Mesh moves the points of a grid (1 to 32 columns and rows). Starting a drag with `Ctrl` held gives a free transform. In Perspective, moving a corner moves the other corner on the same edge the opposite way horizontally.

Liquify is chosen from the toolbar (it has no key). It moves pixels like a brush. In the sub tool list choose Push, Twirl Right, Twirl Left, Pinch, Expand or Restore, and set Diameter and Strength. Restore goes back to the pixels from when you started this liquify.

Moving a text layer changes the text's position, rotation and size. Flipping, scaling with a different aspect ratio, skewing and transforming only inside a selection are not possible ([GUIDE_PAINT.md](GUIDE_PAINT.md)).

## Copy, cut and paste

They are also in the Edit menu.

| Action | Key |
|---|---|
| Copy | `Ctrl+C` |
| Cut | `Ctrl+X` |
| Copy Merged | `Ctrl+Shift+C` |
| Paste | `Ctrl+V` |

Copy and Cut take the pixels of the selection (the whole layer if there is no selection) from the painting channel (the mask, if you are painting a mask). Copy Merged copies what you see, the composite. The same pixels are also written to the OS clipboard as an image, so you can paste them into other apps. Paste makes a new layer (one undo step), and it can paste an image copied in another app. An image larger than the texture set cannot be pasted.
