# Painting

[日本語](../GUIDE_PAINT.md)

This page covers brushes and the eraser, pen pressure, lines, shapes and rulers, symmetry, Fill and Gradient, colors, the stencil and the Text tool. For fill layers and painting with materials, see [GUIDE_FILL.md](GUIDE_FILL.md). For paths, see [GUIDE_PATHS.md](GUIDE_PATHS.md). For painting in the 3D view, see [GUIDE_3D.md](GUIDE_3D.md).

## Painting with a brush

The brush (`B`) and the eraser (`E`) paint on the 2D Canvas and on the 3D View. The eraser uses the eraser you last selected in the eraser list. What they paint on is the painting channel of the layer selected in the Layers panel (or its mask, if the mask is selected).

In Tool Properties in the Tools panel on the left, you can change Size, Opacity, Hardness, Flow, Spacing and Stabilizer right away. `[` and `]` also change the size. Brush Size below it is a set of preset diameters (shown only for the brush, the eraser and the Selection Pen). The adjustment button (Brush Details) opens a window that holds all the settings, grouped as follows. You can move it and keep it open while you paint.

| Category | Contents |
|---|---|
| Shape | The tip (round, a built-in image, or an imported image), hardness, anti-aliasing (None, Weak, Medium, Strong; smooths the jagged pixels of the edge), roundness, angle, follow direction, flip |
| Stroke | Size, spacing, flow, opacity, stabilizer, curve. The 3D settings are in [GUIDE_3D.md](GUIDE_3D.md) |
| Pen Pressure | For each of size, opacity, flow and hardness: whether pressure is used, its minimum, and its curve |
| Taper & Pen | Taper, fade, pen tilt and rotation, speed |
| Jitter, Texture, Dual Brush, Color Dynamics | Scatter, paper texture, a second tip layered on the first, changes in hue, saturation and brightness |
| Color Mixing, Effect | Picking up and mixing the colors underneath (used by the oil, gouache and mixer brushes); Blur, Smudge and Clone |
| Symmetry | See “Painting with symmetry” below |

The settings are specified in [BRUSH.md](../BRUSH.md) (Japanese).

The Clone brush copies from the point you `Alt`-click (release without moving), starting with the next stroke. This works the same on the 2D Canvas and in the 3D View (if you move before releasing, the 2D Canvas rotates the view and the 3D View snap-orbits). A cross marks the source; on the 2D Canvas it moves to the point being copied while you paint. With Aligned on, the offset to the source is set at the start point of the next stroke after you set the source, and later strokes keep the same offset (a stroke cancelled with `Esc` or a stroke that changed nothing does not set it; the next stroke does). The offset that was set carries on after switching to the eraser and back or choosing the brush again. With Aligned off, every stroke starts on the source. All layers reads the stack of visible layers. These two settings are shared by 2D and 3D. Switching documents or texture sets forgets the source. On the 2D Canvas without a source, the brush copies with Offset X and Offset Y under Effect. Once a source is set on the 2D Canvas, these fields show the offset that was set, and an offset set from the source does not mark the brush as modified (an offset you change in the fields does). With Aligned off, and while only the 3D View is shown, the fields do not decide the offset and are dimmed.

With any tool selected, the right button temporarily acts as the eyedropper: in 2D, press it (while you hold it, the swatch follows the pointer, and the color where you release is picked); in 3D, release it without moving. With Polygon Fill, the right button opens the island menu (`Alt` is a view control). While you paint, the controls in the panels do nothing when pressed. They keep the look they had before the stroke and do not turn gray.

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

- **Pressure**: the pen button in the Size, Opacity, Hardness and Flow rows of Tool Properties turns pressure on or off for that setting. The minimum (the value at zero pressure) and the curve are set in Pen Pressure in Brush Details. When a finger or pen arrives as touch input and sends a force, that force becomes the pressure (on the 2D Canvas and in the 3D View alike, through the Pen Pressure adjustment below).
- **Differences between pens**: Pen Pressure under Pen in Edit → Settings… sets a low limit, a high limit and a curve. Draw a few strokes at your usual strength and press Auto, and the settings are worked out from the pressure of the strokes you drew. This is a per-device setting and is stored neither in documents nor in brushes.
- **Tablets on Windows**: on Windows, the pen's pressure, tilt, rotation, eraser end and side buttons are read through Windows Ink by default. On a machine where a driver such as Wacom's provides WinTab, you can switch to WinTab with Pen input under Pen in Edit → Settings… (when WinTab is unavailable, Windows Ink stays in use).
- **Tablets on Mac**: on Mac, the app reads the pressure, tilt, eraser end and side buttons that drivers such as Wacom and XP-Pen send as standard macOS events (experimental). Turn it on or off with Tablet pressure (experimental) under Pen in Edit → Settings… (on by default; when off, the pen draws like a mouse).
- **Delay before a line appears**: by default, the screen update is presented without waiting for the vertical sync, which shortens the delay between the pen input and the line appearing on screen. If screen tearing bothers you, turn on VSync under Display in Edit → Settings… (the delay gets longer; it takes effect from the next start).
- **Stabilizer**: Stabilizer in Tool Properties (0 to 200 px) is the length of the string that pulls the brush. 0 turns it off.
- **Taper**: the settings that thin the start and end of a stroke are in Taper & Pen in Brush Details.
- **In the 3D View too**: Stabilizer, taper, jitter and the other settings apply to strokes in the 3D View with the same formulas. The stabilizer and taper lengths (px) are measured in screen points in the 3D View ([GUIDE_3D.md](GUIDE_3D.md)).

## Lines, shapes and rulers

- **Straight lines**: with the brush, the eraser or an effect brush, `Shift`-click to draw a straight line from the end of the previous stroke to the clicked point (one undo step). If you keep dragging, the rest of the stroke is drawn normally. When there is no previous point (after switching documents or texture sets), the stroke starts at the pressed point, and if you hold `Shift` and drag, the first direction you move in is locked to a multiple of 45°. This works the same with the mouse and the pen, on the 2D Canvas and in the 3D View. In the 3D View, the end of the previous stroke (the last point that hit a surface) is remembered as a point on the surface, and the line is drawn on the screen from where that point appears with the current camera to the clicked point (only the faces visible in the current projection are painted). After the model changes, and after a stroke that never hit a surface, there is no previous point. A contact with `Ctrl` held does not paint with the pen but paints with the mouse (the same in 2D and 3D).
- **Shape** (`U`): drag to draw a Line, Rectangle or Ellipse. Outline draws the outline as one stroke with the current brush, and Fill fills the inside with the paint color (only inside the selection, if there is one). A rectangle has a Corner Radius. `Shift` gives 45° steps, a square or a circle, `Alt` pressed after you start dragging draws from the center, and `Esc` cancels. Releasing makes one undo step. It works in the 3D View too, where the shape you drag on the screen is projected onto the visible faces ([GUIDE_3D.md](GUIDE_3D.md)).
- **Ruler** (`Shift+U`): drag to place a Straight Ruler, Parallel, Concentric or Perspective (1 or 2 points) ruler. Drag an endpoint or a line to move it, and use Delete to remove it. On the 2D Canvas each texture set has one ruler, drawn as a thin line over the view. The 3D View can hold one ruler of its own, which stays at the same place on the screen when you move the viewpoint. The kind, the number of points and Delete apply to both the 2D and the 3D ruler. A ruler is view state and is not saved in the `.ylp`.
- **Snap** (Snap to Ruler, `Ctrl+1`): while on, the points of brush and eraser strokes (including `Shift` lines) are pulled onto the ruler's lines (strokes in the 3D View are pulled onto the 3D View's ruler, on the screen). When combined with `Shift`, the ruler takes priority.

## Painting with symmetry

Set it in Symmetry in Brush Details, or in the symmetry menu on the options bar. Both the 2D and the 3D symmetry work whether you paint on the 2D canvas or in the 3D view. With both on, the 3D copies are made first, and then the original and each 3D copy are copied by the 2D symmetry (the number of copies multiplies). All copies are part of the same stroke, so one undo takes them back. Symmetry cannot be combined with the Smudge and Clone effect brushes (with either symmetry on, they do not start). The brush and the eraser in Quick Mask do not use symmetry.

- **2D**: choose Vertical, Horizontal, Both or Radial (2 to 16 copies). Center X and Center Y move the point where the axes cross, and Canvas center puts it back. Show axes draws the axes on the Canvas. When you paint in the 3D view, the texture pixels you paint are copied across the same axes on the canvas (the copy is made on the UV plane, not on the model).
- **3D**: while a model is open, a "3D" group appears below "2D". Mirror reflects across a plane perpendicular to a model X, Y or Z axis. Place the plane with the Center value or the Origin and Bounds center buttons. Radial rotates copies around a model axis (it can be combined with the mirror). In the 3D view, Ignore visibility also paints the copies on faces the camera cannot see. Show plane draws the plane and axis in the 3D view. When you paint on the 2D canvas, the surface point under the UV at the center of the brush is copied, and the same shape is placed at the UV of the face it lands on (following the texture density and the UV orientation there). Where faces share the same UVs, the copies are made from every one of those faces. Brush marks on UV areas that belong to no face are not copied. The 2D canvas has no viewpoint, so the copies are painted regardless of Ignore visibility. A copy that does not reach a face, or that falls on another texture set, is skipped and you are told. On the 2D canvas, a copy onto a face whose UVs are flat or whose texture density is far off is skipped too, and you are told.

## Filling

- **Fill** (`G`): fills where you press. The sub tool sets the extent: Similar colors, Triangle, Mesh Part, UV Island or Material. Choose Paint or Erase; if there is a selection, only the inside is affected. Similar colors has Tolerance, Contiguous, a Reference (the editing layer, all visible layers, or reference layers), Perceptual difference, Close gap, Area scaling and Paint unfilled areas. In the 3D View, it works out the same extent as the 2D Canvas from the pixel that the UV of the pressed surface points to (Paint unfilled areas uses the line through the UV pixels of the surfaces you trace, and does not join across places off the surface or across UV seams).
- **Polygon Fill** (`4`): drag to add the extents you pass over, and release for one undo step. The extent types are the same as the Fill tool's (without Similar colors). For overlapping UVs, see [GUIDE_3D.md](GUIDE_3D.md).
- The extent under the pointer is highlighted by faces in 3D and by the UV outline in 2D.
- **Gradient** (`Shift+G`): drag on the 2D Canvas or in the 3D View to paint a Linear or Radial gradient from the start (the paint color) to the end (transparent, or the sub color). It paints the painting channel of the selected layer, or the mask when you are painting a mask (White (show) and Black (hide)). If Paint several channels at once is on in the Material tab of Properties, it paints every channel in the set, and Between two materials lets the end be another material ([GUIDE_FILL.md](GUIDE_FILL.md)). In the 3D View, each pixel of the visible faces takes the color of the place where it appears on the screen ([GUIDE_3D.md](GUIDE_3D.md)).

## Choosing and picking colors

- **Setting the color**: use the main and sub colors at the bottom of the toolbar, or the Color panel. Color has a hue wheel or a square and hue bar (switch with the button at the top right), a hex field (`#RRGGBB`) and alpha. `X` swaps the main and sub colors, and `D` returns to the default colors.
- **Color window**: pressing the color swatch of a value such as a fill layer's color, a lilToon color or a gradient color opens a color window. Set the color on the spot with the hue wheel, hex, opacity or a color from a color set. Use Paint Color puts in the current paint color. It does not close when you click outside, so you can change colors while working in other fields. `Esc` restores the color from when it opened and closes it.
- **Color sets**: the Color Sets panel manages sets of colors (new, duplicate, rename, delete, import GPL and ACO, export a GIMP palette). Click a color for the main color, or `Alt`-click for the sub color. It also shows a History and the Intermediate Colors between four corner colors.
- **Eyedropper** (`I`): picks the value at the pressed point on the 2D Canvas or the 3D View as the paint color of the painting channel. While it picks, the pointer shows an eyedropper icon and a ring (the top half is the current color and the bottom half is the color under the pointer; a scalar channel is a gray level). It reads only the selected layer; turn on Sample All Layers in the options bar to read the composite of all layers. With Paint several channels at once on, it picks the six standard channels into the brush material's values.
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
- **Change later**: Font, Size, Color, Line Spacing, Tracking, alignment (Left, Center, Right) and Wrap Width can be changed in Tool Properties, the options bar, or Text in the layer's Properties. Choose a font from the bundled BIZ UDPGothic (regular and bold), Installed Fonts, or From File… (`.ttf`, `.otf`, `.ttc`).
- **Text color**: under Text Color in Tool Properties, choose where the color comes from. Paint Color (the default) makes new text in the paint color, and while the Text tool is in use, every change of the paint color changes the selected or typed text to the same color (dragging the color wheel is one undo step). Choosing a paint color with another tool, such as the brush, does not change the text that stays selected. A locked layer is not changed. The Color field then shows the paint color, and pressing it opens the color window for the paint color. Tool Color makes new text in the color of the field, and changing the field changes the text (it does not follow the paint color). The choice is remembered only while the app is open. Text has one color per layer; colors per character are not supported. The Color field in Text in the layer's Properties shows and changes only that layer's color.
- **Move / Transform**: changes the text layer's position, rotation and size. Flipping, scaling with a different aspect ratio, skewing and transforming only inside a selection are not possible.
- **Rasterize**: to make it a layer you can paint on, use Rasterize Text in the Layer menu or the right-click menu. It keeps the current pixels and removes the text. Text is drawn in the Color channel.
- **Fonts are not stored in the `.ylp`**. Only the file location, names and the SHA-256 of the contents are remembered. A text layer whose font cannot be found when you open the document keeps its drawn pixels and refuses retyping. When a font with the same name but different contents is found, you are told, and retyping redraws it with that font.

A document with a text layer cannot be opened by 0.4.x. A PSD export writes it as a pixel layer. The format is in [YLP_FORMAT.md](../YLP_FORMAT.md) (Japanese).
