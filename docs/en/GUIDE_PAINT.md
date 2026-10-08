# Painting

[日本語](../GUIDE_PAINT.md)

This page covers brushes and the eraser, pen pressure, lines, shapes and rulers, symmetry, Fill and Gradient, colors, the stencil and the Text tool. For fill layers and painting with materials, see [GUIDE_FILL.md](GUIDE_FILL.md). For paths, see [GUIDE_PATHS.md](GUIDE_PATHS.md). For painting in the 3D view, see [GUIDE_3D.md](GUIDE_3D.md).

## Painting with a brush

The brush (`B`) and the eraser (`E`) paint on the 2D Canvas and on the 3D View. The eraser uses the eraser you last selected in the eraser list. What they paint on is the painting channel of the layer selected in the Layers panel (or its mask, if the mask is selected).

In Tool Properties in the Tools panel on the left, you can change Size, Opacity, Hardness, Flow, Spacing and Stabilizer right away. `[` and `]` also change the size. Brush Size below it is a set of preset diameters (shown only for the brush, the eraser and the Selection Pen). The adjustment button (Brush Details) opens a window that holds all the settings, grouped as follows. You can move it and keep it open while you paint.

| Category | Contents |
|---|---|
| Shape | The tip (round, a built-in image, or an imported image), hardness, roundness, angle, follow direction, flip |
| Stroke | Size, spacing, flow, opacity, stabilizer, curve. The 3D settings are in [GUIDE_3D.md](GUIDE_3D.md) |
| Pen Pressure | For each of size, opacity, flow and hardness: whether pressure is used, its minimum, and its curve |
| Taper & Pen | Taper, fade, pen tilt and rotation, speed |
| Jitter, Texture, Dual Brush, Color Dynamics | Scatter, paper texture, a second tip layered on the first, changes in hue, saturation and brightness |
| Color Mixing, Effect | Picking up and mixing the colors underneath (used by the oil, gouache and mixer brushes); Blur, Smudge and Clone |
| Symmetry | See “Painting with symmetry” below |

The settings are specified in [BRUSH.md](../BRUSH.md) (Japanese). In 2D, with Brush, Eraser, Fill or Polygon Fill selected, `Alt`-click temporarily acts as the eyedropper (`Alt` orbits in 3D). While you paint, the controls in the panels do nothing when pressed. They keep the look they had before the stroke and do not turn gray.

## Choosing and rearranging brushes

Choose a brush from the list at the top of the Tools panel. It has a tab for each group (initially Pen, Brush, Airbrush, Effects and Special, plus Imported when you have imported brushes), and each row shows the brush's name and a sample stroke drawn with its settings. The eraser has its own groups, separate from the brushes. A brush whose settings you changed is marked “modified”.

- The buttons below the list revert to the original settings, duplicate, add the current settings as a new brush, and delete. Double-click a row to rename it.
- Add Brushes (the “+” button) opens a window that lists built-in, bundled Krita, Photoshop, CLIP STUDIO, imported and your own brushes by kind, with a name search. The ones you select are placed in the current group.
- Import brushes from other apps (ABR, GBR, GIH, VBR, PNG, PAT) with Import brushes, and CLIP STUDIO brushes with Import from CLIP STUDIO. You can also drop the files onto the window (a PNG only when you drop it on the brush list; see [BRUSH_IMPORT.md](../BRUSH_IMPORT.md), in Japanese).

### Rearranging the toolbar and groups

Tools in the toolbar on the left, group tabs and brush rows can be reordered by dragging (`Esc` cancels). Release a group tab or a brush row over a Brush or Eraser tool in the toolbar to move it to that tool. Hold `Ctrl` when releasing a brush row to place a copy. The right-click menus are:

| Where | Menu |
|---|---|
| Toolbar | Add Tool, Delete This Tool, Rename, Change Icon, Separator Before, Reset Tool Layout… |
| Group tab | Add Group, Rename, Duplicate, Delete |
| Brush row | Rename, Duplicate, Register These Settings, Revert, Delete |

Add Tool lists Brush, Eraser, and the built-in tools you removed from the toolbar. You can have any number of Brush and Eraser tools, and each has its own groups, so you can make a tool out of just the brushes you like. The last one of each cannot be deleted. Deleting only removes an item from the layout, and the brush files stay. The layout is saved in `tools.json` in the settings folder and is not stored in documents. Your own brushes are in `brushes/` in the same folder, and damaged files are skipped and reported at startup. For limits and the file format, see [SUBTOOLS.md](../SUBTOOLS.md) (Japanese).

## Tuning how the pen feels

- **Pressure**: the pen button in the Size, Opacity, Hardness and Flow rows of Tool Properties turns pressure on or off for that setting. The minimum (the value at zero pressure) and the curve are set in Pen Pressure in Brush Details.
- **Differences between pens**: View → Pen Pressure… sets a low limit, a high limit and a curve. Draw a few strokes at your usual strength and press Auto, and the settings are worked out from the pressure of the strokes you drew. This is a per-device setting and is stored neither in documents nor in brushes.
- **Tablets on Mac**: on Mac, the app reads the pressure, tilt, eraser end and side buttons that drivers such as Wacom and XP-Pen send as standard macOS events (experimental). Turn it on or off with Tablet pressure (experimental) under Pen in Edit → Settings… (on by default; when off, the pen draws like a mouse).
- **Stabilizer**: Stabilizer in Tool Properties (0 to 200 px) is the length of the string that pulls the brush. 0 turns it off.
- **Taper**: the settings that thin the start and end of a stroke are in Taper & Pen in Brush Details.
- **With the Canvas and the 3D View side by side**: settings that affect only 2D strokes, such as Stabilizer, taper and jitter, can still be changed (they apply to strokes on the Canvas). When only the 3D View tab is showing, they are grayed out and the tooltip says why.

## Lines, shapes and rulers

- **Straight lines**: with the brush, the eraser or an effect brush, `Shift`-click to draw a straight line from the end of the previous stroke to the clicked point (one undo step). When there is no previous point (after switching documents or texture sets, or after an undo), you start a normal stroke. If you hold `Shift` and drag, the direction is locked to a multiple of 45°. 2D Canvas only.
- **Shape** (`U`): drag to draw a Line, Rectangle or Ellipse. Outline draws the outline as one stroke with the current brush, and Fill fills the inside with the paint color (only inside the selection, if there is one). A rectangle has a Corner Radius. `Shift` gives 45° steps, a square or a circle, `Alt` draws from the center, and `Esc` cancels. Releasing makes one undo step. 2D Canvas only.
- **Ruler** (`Shift+U`): drag to place a Straight Ruler, Parallel, Concentric or Perspective (1 or 2 points) ruler. Drag an endpoint or a line to move it, and use Delete to remove it. Each texture set has one ruler, drawn as a thin line over the view. A ruler is view state and is not saved in the `.ylp`.
- **Snap** (Snap to Ruler, `Ctrl+1`): while on, the points of brush and eraser strokes (including `Shift` lines) are pulled onto the ruler's lines. When combined with `Shift`, the ruler takes priority.

## Painting with symmetry

Set it in Symmetry in Brush Details, or in the symmetry menu on the options bar. It cannot be combined with the Smudge and Clone effect brushes.

- **2D**: choose Vertical, Horizontal, Both or Radial (2 to 16 copies). Center X and Center Y move the point where the axes cross, and Canvas center puts it back. Show axes draws the axes on the Canvas.
- **3D**: Mirror reflects across a plane perpendicular to a model X, Y or Z axis. Place the plane with the Center value or the Origin and Bounds center buttons. Radial rotates copies around a model axis (it can be combined with the mirror). Ignore visibility also paints the copies on faces the camera cannot see. Show plane draws the plane and axis in the 3D view. A copy that does not reach a face, or that falls on another texture set, is skipped.

## Filling

- **Fill** (`G`): fills where you press. The sub tool sets the extent: Similar colors (2D only), Triangle, Mesh Part, UV Island or Material. Choose Paint or Erase; if there is a selection, only the inside is affected. Similar colors has Tolerance, Contiguous, a Reference (the editing layer, all visible layers, or reference layers), Perceptual difference, Close gap, Area scaling and Paint unfilled areas.
- **Polygon Fill** (`4`): drag to add the extents you pass over, and release for one undo step. The extent types are the same as the Fill tool's (without Similar colors). For overlapping UVs, see [GUIDE_3D.md](GUIDE_3D.md).
- The extent under the pointer is highlighted by faces in 3D and by the UV outline in 2D.
- **Gradient** (`Shift+G`): drag on the 2D Canvas to paint a Linear or Radial gradient from the start (the paint color) to the end (transparent, or the sub color). It paints the painting channel of the selected layer, or the mask when you are painting a mask (White (show) and Black (hide)). If Paint several channels at once is on in the Material tab of Properties, it paints every channel in the set, and Between two materials lets the end be another material ([GUIDE_FILL.md](GUIDE_FILL.md)). Not available in the 3D View.

## Choosing and picking colors

- **Setting the color**: use the main and sub colors at the bottom of the toolbar, or the Color panel. Color has a hue wheel or a square and hue bar (switch with the button at the top right), a hex field (`#RRGGBB`) and alpha. `X` swaps the main and sub colors, and `D` returns to the default colors.
- **Color window**: pressing the color swatch of a value such as a fill layer's color, a lilToon color or a gradient color opens a color window. Set the color on the spot with the hue wheel, hex, opacity or a color from a color set. Use Paint Color puts in the current paint color. It does not close when you click outside, so you can change colors while working in other fields. `Esc` restores the color from when it opened and closes it.
- **Color sets**: the Color Sets panel manages sets of colors (new, duplicate, rename, delete, import GPL and ACO, export a GIMP palette). Click a color for the main color, or `Alt`-click for the sub color. It also shows a History and the Intermediate Colors between four corner colors.
- **Eyedropper** (`I`): picks the value at the pressed point on the 2D Canvas or the 3D View as the paint color of the painting channel. It reads only the selected layer; turn on Sample All Layers in the options bar to read the composite of all layers. With Paint several channels at once on, it picks the six standard channels into the brush material's values.
- **Screen color** (Windows only): in the Edit menu, Pick Screen Color (`Ctrl+Alt+I`) and Hide Window and Pick Screen Color (`Ctrl+Alt+Shift+I`) pick a color from the desktop.

## Painting through a stencil

You can paint through a translucent image overlaid on the screen, on the 2D Canvas and in the 3D View alike. It also works with the effect brushes (Blur, Smudge, Clone). The settings are in the Stencil tab of Properties.

- Image: Load an Image… reads a PNG (you can also drop a PNG there). Stop using the stencil turns it off.
- Reads as: Amount (mask) uses the image's brightness as how much paint gets through (white paints; black and transparent hold back). Color paints with the image's colors. Auto uses amount for a gray image and color otherwise. Invert applies to amount.
- Tiling (No tiling, Horizontal, Vertical, Both), Overlay opacity, Size and Angle.
- Hold `Y` and left-drag to rotate (`Shift` for 15° steps), middle-drag or `Ctrl` + left-drag to move, and right-drag or `Alt` + left-drag to resize (in 2D and 3D). Reset placement undoes this. The stencil stays fixed to the screen. Hold `N` to bypass the stencil.

The stencil is application state; it is stored neither in the `.ylp` nor in brush presets.

## Adding text

The Text tool (`T`) types from the point you press on the 2D Canvas. The first character adds a text layer. `Enter` starts a new line, and `Esc` finishes typing. Pressing outside the box finishes typing and lets you start new text there. Japanese input conversion works. What you typed is one undo step, and if no text remains, no layer is kept.

- **Edit again**: select the text layer and press inside its box, or double-click the box with Move / Transform (`V`).
- **Change later**: Font, Size, Color, Line Spacing, Tracking, alignment (Left, Center, Right) and Wrap Width can be changed in Tool Properties, the options bar, or Text in the layer's Properties. Choose a font from the bundled BIZ UDPGothic (regular and bold), Installed Fonts, or From File… (`.ttf`, `.otf`, `.ttc`). New text uses the paint color.
- **Move / Transform**: changes the text layer's position, rotation and size. Flipping, scaling with a different aspect ratio, skewing and transforming only inside a selection are not possible.
- **Rasterize**: to make it a layer you can paint on, use Rasterize Text in the Layer menu or the right-click menu. It keeps the current pixels and removes the text. Text is drawn in the Color channel.
- **Fonts are not stored in the `.ylp`**. Only the file location, names and the SHA-256 of the contents are remembered. A text layer whose font cannot be found when you open the document keeps its drawn pixels and refuses retyping. When a font with the same name but different contents is found, you are told, and retyping redraws it with that font.

A document with a text layer cannot be opened by 0.4.x. A PSD export writes it as a pixel layer. The format is in [YLP_FORMAT.md](../YLP_FORMAT.md) (Japanese).
