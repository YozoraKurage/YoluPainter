# 3D View and Models

[日本語](../GUIDE_3D.md)

This page covers viewing a model in the 3D view, moving around it, and painting on its surface. Opening an FBX, posing, baking mesh maps, overlapping UVs and opening from Unity are here too.

## Move around the 3D view

Rotate with a right drag. An `Alt`+left drag also rotates, and when the direction comes within 15° of an axis view (front, back, right, left, top, bottom) it snaps to that view. While the right button is held, `W` / `S` move the view forward and back, `A` / `D` left and right, and `Q` / `E` down and up (`Shift` for faster). Pan with a middle drag or `Space`+left drag. Zoom with the wheel or `Ctrl+Space`+left drag. Releasing the right button without moving it picks the value of that face as the paint color (eyedropper; with Polygon Fill it opens the island menu). `.` over the 3D view fits the faces of the current texture set, and "Fit the whole model in view" in the top right fits the whole model. All the keys are in [GUIDE_KEYS.md](GUIDE_KEYS.md).

### Axis views and orthographic

Below the icons in the top right of the 3D view is the axis widget (red X, green Y and blue Z balls). Pressing a lettered ball looks at the model from that axis's side (X is the Right view, Y the Top view, Z the Front view; the small unlettered balls are the opposite Left, Bottom and Back views). Pressing the ball in the middle switches between perspective and orthographic (it is filled white while the view is orthographic). Dragging on the widget rotates the view. The widget is hidden while the light, environment and tone mapping settings panel is open. The View Pie Menu also has the Front, Back, Right, Left, Top and Bottom views and Toggle Orthographic (the pie has no key at first; see "Change shortcuts" in [GUIDE_KEYS.md](GUIDE_KEYS.md)).

Choosing an axis view, or snapping onto an axis with an `Alt`+left drag, makes the view orthographic. Rotating off the axis brings back perspective. A projection you switched with the middle ball or the pie menu stays when you rotate. Turn off "Orthographic on axis views" under "3D View" in the settings window to keep perspective on axis views as well.

In orthographic, near and far parts look the same size, and the brush circle paints at the same size on the surface at any depth. Painting, picking colors, selecting and placing path points land where you see them in orthographic too. Zooming with the wheel changes how much of the scene is in view, and `W` / `S` while the right button is held zoom in and out instead of moving forward and back. Switching back to perspective keeps the size you zoomed to. The view (its direction and projection) is not saved in the file.

## Open a model

While there is no model, the 3D view shows "New Project…" (the same as New Project in the File menu) and "Load Test Cube". Open a model with "Choose a model…" in New Project and Project Configuration, "Open FBX" in the Pose panel, or by dropping a .fbx onto the window. To open from Unity, see Live Link at the end of this page.

Each material of the model gets a texture set, and you switch between them in the Texture Sets panel. You can paint only on the faces of the current set's material.

## Change how the model looks

There are three icons in the top right of the 3D view.

- "3D shading": choose Material (PBR) (the same BRDF as Unity Standard, showing Roughness, Metallic, Normal and Emission), Neutral, Channel Only (Color, Roughness, Metallic, Normal, Emission, Height) or Mesh Map Only (the baked maps).
- "Light, environment, tone mapping": opens the settings panel.
- "Fit the whole model in view".

The settings panel has three pages.

- "Display": Environment (None, Sky, Studio, with Intensity, Rotation, Show as background and Blur), Light (Strength, Azimuth, Elevation, Shadows and Softness), Tone Mapping (None, Neutral, ACES, and Exposure), and "Reset". A light strength of 1 is as bright as a white, strength-1 directional light in Unity.
- "Quality": anti-aliasing and bloom.
- "Navigation": the orbit center and the zoom center.

The display textures sent to the 3D view are padded beyond the UVs, so island edges do not bleed when you look from a distance (saving and exports are unchanged). When the 3D picture is shown smaller, for example because of the GPU memory budget, a mark appears in the top right. [PREVIEW.md](../PREVIEW.md) (in Japanese) has the details of the look and its speed.

In the Material panel in the right dock, "Look" chooses "Standard (PBR)" or "lilToon" for each texture set. lilToon draws a reproduction of the lilToon look, and its color, normal map, reflection and advanced values can be edited in the fields. "lilToon Template" creates and assigns the user channels for the masks of the features you turned on.

The material values received from Unity appear as "Unity Values", and "Match Unity" drops the values changed here and draws with Unity's. A shader with only a similar name is not drawn as lilToon ([PREVIEW.md](../PREVIEW.md)).

## Paint on the model

You paint on the model in Paint mode ("Modes" in [GUIDE_START.md](GUIDE_START.md)). The tools that work in the 3D view are Brush, Eraser, Eyedropper, Fill, Polygon Fill, Rectangle Select, Ellipse Select, Lasso, Polygon Select, Magic Wand, Selection Pen, ID Color Select, Path, Gradient, Shape and Ruler (paths are in [GUIDE_PATHS.md](GUIDE_PATHS.md); Gradient, Shape and Ruler are in "Drag on the screen" below; the four shape selections, Magic Wand and Selection Pen are in "Select by dragging on the screen"). Move / Transform, Liquify and Text work only on the 2D canvas. While Quick Mask is on, the brush and the eraser in the 3D view work as a selection pen and a selection eraser, as on the 2D canvas: they add the texture pixels of the faces you paint to the selection or take them out (Size, Hardness, Opacity and pressure come from the current brush; one stroke is one undo; layer pixels do not change). The red overlay is shown on the faces in the 3D view and on the 2D canvas ([GUIDE_SELECT.md](GUIDE_SELECT.md); the selection on the faces is in "Select by dragging on the screen" below).

Brush settings work in the 3D view with the same formulas as on the 2D canvas: Size, Hardness, Anti-aliasing (its band is measured in texture pixels), Flow, Opacity, pressure, image tips (angle, roundness, Flip X and Flip Y, Follow direction), the stroke (Stabilizer, Curve), Taper & Pen (fade, tilt, rotation, speed), jitter, texture, dual brush, color, Color Dynamics (including Apply per tip), color mixing, erasing, stencils and the effect brushes (Blur, Smudge, Clone).

- The tip shape is laid out on the screen and projected from the camera onto the surface (it is not laid flat along the surface).
- Dabs are placed every spacing along the stroke length, as in 2D. However densely or sparsely the input points arrive, the stroke's density and the length of a fade stay the same. The spacing comes from the brush size converted to the screen at the depth of the surface where each stretch of the stroke starts.
- Stabilizer, taper and speed lengths and speeds are measured in screen points in the 3D view (in document pixels on the 2D canvas).
- The texture is read at the document pixel being painted, so its grain lines up between what you paint in 2D and in 3D.
- 3D symmetry comes from the document's 3D symmetry ruler (in model space). A line symmetry flips the tip across the mirror plane (with four or more lines, copies rotated around the axis as well), and a rotational symmetry rotates the tip around the axis. The 2D symmetry ruler works too, copying the texture pixels you paint across the canvas ("Painting with symmetry" in [GUIDE_PAINT.md](GUIDE_PAINT.md)). How to make a 3D symmetry ruler is under "Ruler" in "Drag on the screen" below.
- If a brush mark does not fit in the memory of one operation, the whole stroke is cancelled, as on the 2D canvas (no part is left unpainted).
- When you move a large brush quickly, painting (and reflecting what was painted in the 3D display) is limited to between 8 and 40 ms per frame so the window does not freeze. When the work that has piled up is expected to take more than about 0.3 s, the limit widens to catch up; otherwise it stays at 8 ms. The paint lags a little behind the pen and keeps going after you lift it until it is done (aim: within 0.5 s; when painting cannot keep up, for example with a very fast pen and a long line or a large document, the lag grows with the length of the line). If you start the next operation while it is still being painted (undo, save, a new stroke, a tool key, the wheel, closing the window and so on), the rest is painted and committed first, and then the operation goes through. `Esc` works the same way: it does not throw away a line you have already lifted the pen from (an `Esc` before you lift still cancels the whole stroke). When it is done, it becomes one undo step.

While the 3D view is shown, the brush properties gain a "3D" group.

| Field | What it does |
|---|---|
| Paint hidden areas | Also paints surfaces hidden behind nearer ones under the brush |
| Paint back faces | Also paints faces that point away from the camera |
| Fade by angle | Paints more lightly where the surface turns away from the view ("Fade start", "Fade end") |
| Seam bleed | How far to paint outside the edges of UV islands so seams do not show |

### Drag on the screen

Gradient and Shape are set by dragging on the screen of the 3D view. When you release, the result is projected onto the faces visible from the camera (nothing changes while you drag; `Esc` cancels). This works the same in orthographic. Ruler is made by pressing on a face of the model and dragging; when you release, a 3D ruler is created ("Ruler" below). The Tool Properties fields are the same as on the 2D canvas.

- **Gradient**: each texture pixel of the visible faces takes the color of the place where it appears on the screen. With Linear, the start side has the start color, the end side has the end color, and in between it blends as in 2D. The selection, painting a mask and painting with materials work as in 2D.
- **Shape, Fill**: paints the visible faces that appear inside the shape on the screen. As with Fill on the 2D canvas, each texture pixel is checked at 4 × 4 points and painted by the share of those points inside the shape (the brush's Anti-aliasing does not change it). As in 2D, symmetry is not used. A rectangle's Corner Radius is measured in screen points.
- **Shape, Outline**: draws the outline as a brush stroke in the 3D view. The brush settings and symmetry apply (the pressure is constant, and Stabilizer and Curve are not used, as in 2D).
- Gradient and Shape Fill never paint hidden areas or back faces, whatever the brush's "3D" group says. Fade by angle and Seam bleed follow the group.
- Gradient and Shape make one undo step when you release. On a locked layer, on a read-only texture set, or when it does not fit in the memory of one operation, they are refused and nothing changes.
- **Ruler**: with the Ruler tool, pressing on a face of the model and dragging creates a 3D ruler on the selected layer (with "Create on Edit Layer" off in the Tool Properties, it is created on a new layer named "Ruler"). A 3D ruler is a value in model space, belongs to the layer and is saved in the `.ylp` (it stays at the same place on the model when you orbit; it is separate from the 2D canvas's ruler). Pressing outside the model creates nothing. The ruler is created when you release, as one undo step (`Esc` cancels).
  - **Straight Ruler, Parallel, Concentric, Perspective**: the pressed point on the face is `a` and the release point is `b` (where no face is hit, a point on the plane through the pressed point that faces the camera); the normal of the pressed face is the ruler's direction. Releasing without dragging creates nothing. Choose 1 or 2 points for Perspective in the Tool Properties.
  - **Symmetry**: dragging makes the first mirror plane the plane through the pressed point that contains the line you drew on the screen and the line of sight (holding `Shift` turns the mirror's normal to the nearest of the model's X, Y and Z axes). Releasing without dragging makes an X mirror through the center of the model's bounds. Lines and Line Symmetry are set in the Tool Properties.
  - A special ruler (Parallel, Concentric, Perspective, Symmetry) is marked when you create it and Snap to Special Ruler is turned on (only one 3D special ruler takes effect, separately from 2D). Switch the Snapping Special Ruler (`Ctrl+4`) steps through the special rulers in the space of the view the pointer is over. If the pointer is over neither view, it uses the space of the view where you last started drawing with a tool (a press with Brush, Shape, Gradient, Fill, Ruler and so on); if there is none, it uses 3D when you can only paint in the 3D view, and 2D otherwise.
  - **How it looks**: the 3D rulers you can see are drawn over the 3D view (Visible In and the eye work as for 2D rulers). A symmetry ruler shows the mirror plane's square and the axis line for a 2-line symmetry; otherwise the axis line and lines from the center on the plane perpendicular to the axis. A straight ruler is a line through a and b with a circle at each end, Parallel is a few lines in its direction, Concentric is three circles, and Perspective is the vanishing point and lines from it. A ruler that is not in effect (a special ruler without the mark, or a straight ruler while Snap to Ruler is off) is faint, and the selected ruler (one just created on the selected layer, or chosen on a row in Properties; a ruler created with "Create on Edit Layer" off is not selected) is orange. 2D rulers are not shown in the 3D view.
  - **Snapping**: while Snap to Ruler and Snap to Special Ruler are on, the points of brush and eraser strokes in the 3D view are pulled onto lines projected to the screen with the camera at the start of the stroke (the camera does not move during a stroke). A straight ruler pulls onto the straight line through the projected a and b, and only when the stroke starts near the line (within about 26 points on the screen). Parallel pulls onto the line through the pressed point on the face in the direction `a`→`b`, and Perspective onto the line from the pressed point to the vanishing point (with 2 points, the one closer to the direction of the first movement). Concentric pulls onto the circle around the axis through `a` (the ruler's direction), at the height of the pressed point on the face (the circle through the pressed point, so it does not jump to a circle away from it), projected to the screen as 256 segments (the circles drawn are at the height of `a`). When the press is outside the model, only a straight ruler pulls. Symmetry does not pull; it copies.

3D Smudge and Clone sample across UV island seams. Set the clone source with an `Alt`-click released without moving (if you move, it is the snap orbit). "Aligned" keeps the offset from the previous stroke, and "All layers" reads the visible layers together. These two are the same settings as on the 2D canvas. Because the 3D source is set by a point on a surface, the clone's "Offset X" and "Offset Y" fields are dimmed while only the 3D view is shown. Switching texture sets forgets the source.

`Shift`-click draws a straight line from the end of the previous stroke (the last point that hit a surface) to the clicked point (with the mouse and the pen; when there is no previous point, holding `Shift` and dragging locks the first direction you move in to 45° steps). A contact with `Ctrl` held does not paint with the pen but paints with the mouse. The force of a touch, such as a finger, becomes the pressure. These also work the same as on the 2D canvas ([GUIDE_PAINT.md](GUIDE_PAINT.md)). The Fill tool's "Similar colors" also fills the same extent as in 2D, from the pixel that the UV of the pressed surface points to.

### Select by dragging on the screen

Rectangle Select (`M`), Ellipse Select (`Shift+M`), Lasso (`L`) and Polygon Select (`Shift+L`) are set by drawing the shape on the screen of the 3D view. When you release (for a polygon, when it closes with `Enter`, by clicking the first point, or by double-clicking), the shape is projected onto the texture pixels of the faces visible from the camera and becomes the same selection as on the 2D canvas (nothing changes while you drag). This works the same in orthographic. Magic Wand (`W`) runs the same Magic Wand as on the 2D canvas, starting from the texture pixel of the face you press.

- Hidden areas and faces that point away from the camera are not selected, whatever "Paint hidden areas" and "Paint back faces" in the brush's "3D" group say. "Fade by angle" and "Seam bleed" follow the group (faded areas get a lighter amount, and bled pixels are selected).
- The edge is checked at 4 × 4 points per texture pixel, and the amount is the share of those points inside the shape (the same as Ellipse, Lasso and Polygon on the 2D canvas). A rectangle has a smooth edge only with a Corner radius (measured in screen points); the other shapes follow Anti-alias.
- New, Add, Subtract and Intersect (on the options bar and with `Shift` and `Ctrl`), and Fixed ratio and From center (including `Shift` and `Alt` pressed after you start) work as on the 2D canvas. Releasing without moving deselects when the mode is New.
- The points of a lasso and a polygon stay at the same place on the screen when you orbit the view (they do not stick to the surface). While placing a polygon, `Backspace` and `Ctrl+Z` remove the last point.
- As on the 2D canvas, Magic Wand selects the area of similar color connected on the texture, starting from the pressed pixel. It does not look at how faces connect on the model, so it also takes pixels of faces you cannot see and pixels beyond a UV seam whenever they are connected on the texture with a similar color. If the pressed place does not hit the model, it selects nothing and tells you why.
- `Esc`, the window losing focus, or switching tools discards the shape you are drawing and the points you placed without selecting anything. When a release is lost (released outside the window, or the OS took the pen press), nothing is selected either (the 2D canvas selects at the last position).
- One selection is one undo step, and the selection is saved in the `.ylp` for each texture set. On a read-only texture set it is refused at the press, and when it does not fit in the memory of one operation it is refused without changing the selection.
- The Selection Pen and Selection Eraser (`S`) paint the texture pixels of the face you press in the 3D view to add to or remove from the selection (the same path as the brush and eraser while Quick Mask is on). Size, Hardness, Opacity and pressure come from the current brush, and the size on the surface follows the same rule as the 3D brush (a share of the model's size). Add or remove is chosen by `Shift` (add), `Ctrl` (remove), the pen's eraser end and the Selection Eraser option, as on the 2D canvas. Symmetry is not used. One stroke is one undo step. `Esc` discards it, and when the window loses focus or the OS takes the pen press, the stroke so far is selected (the same as the 3D brush).
- The selection is also shown on the visible faces in the 3D view as an edge (white and black dashes), on the faces of the current texture set only. The dashes stay still (they do not march as in 2D). While Quick Mask is on, a red overlay is shown instead of the edge, and while the Selection Pen paints, a blue overlay (adding) or an orange one (removing) is shown too. The line keeps its thickness when you zoom in or out. The edge is drawn on the texture, so even for a selection that continues across a seam on the model, a line appears at the seam when the pixels of the neighboring island are not selected (the 3D selection tools also select the pixels of the seam bleed, so no line appears within the bleed width). The edge and the overlays use GPU memory of the document's size and count against the 3D picture budget (GPU memory in Settings). When they do not fit (or when the document is wider or taller than the GPU texture limit), they are not shown, and the warning icon in the corner of the 3D view says so.

## Select and move objects (Edit mode)

In Edit mode, the 3D view shows a marker (a white dot) for each object on the visible layers. The objects are fill projection boxes and decals (none when the projection is UV), gradient decals, filter shapes, points of point gradients in model space, and 3D paths (drawn on the current model), and 3D rulers (those whose eye is on; their markers show regardless of Visible In).

- Click a marker to select that object and its layer. The selected object's marker, frame or line turns orange. Shapes also show the same handles as when you edit them from the fill or filter fields (`Q` hides them).
- When markers overlap, click the same place again to select the next object. Click an empty spot to deselect.
- `G` (move), `R` (rotate) and `S` (scale) start at the pointer and follow how far the mouse moves. `X`, `Y` and `Z` choose an axis (press again for the object's own axis; scaling uses the object's own axis from the first press), and you can type a value. Left click or `Enter` applies; right click or `Esc` cancels. All the keys are in "Edit and Pose modes" in [GUIDE_KEYS.md](GUIDE_KEYS.md).
- Without keys, choose Move, Rotate or Scale in the tool strip on the left and drag from a marker with the left button (releasing applies it; `X`, `Y`, `Z` and `Ctrl` snapping work during the drag; a pen works too). When you start with a key, that tool in the strip lights up as well.
- Each move, rotation or scale is one undo step. While you move, a coarse preview follows.
- Points can only be moved. A 3D path moves as a whole, and its points are placed back onto the nearest surface points when you apply. Only surfaces facing the same way as where it was drawn are used, so a path on the front of a thin plate does not move to the back (it cannot be applied while a point is too far from the surface).
- `Alt+G`, `Alt+R` and `Alt+S` reset a shape's position, rotation and size to the placement it gets when it is created (fitted to the model's bounds).
- A 3D ruler's marker is at the middle of its two points (Straight Ruler, Parallel, 2-point Perspective) or at its center (`a` for Concentric, Symmetry and 1-point Perspective). When you select one, its lines turn orange and it shows the move arrows and rotation rings (no size knobs) and squares at its end points. The end squares are `a` and `b` for Straight Ruler, Parallel and 2-point Perspective, and `b` (the radius, the first line) for Concentric and Symmetry; dragging one moves it onto the face under the pointer, or, where no face is hit, onto the plane through the previous point that faces the camera. `G`, `R` and `S`: move moves `a` and `b`, rotate turns `a`, `b` and the direction around the center, and scale changes the distance of `a` and `b` from the center (the ruler's direction does not change; scaling along one axis stretches `a` and `b` along it, so the direction from `a` to `b` changes). The second press of an axis key is the ruler's own axis (X is `a`→`b`, Y is the ruler's direction, Z is perpendicular to both). Each is a continuing change, and applying it is one undo step. `Alt+G` puts the center at the center of the model's bounds, `Alt+R` turns the direction to the model's axes (the ruler's direction to the nearest axis, `a`→`b` to the next nearest), and `Alt+S` sets the distance from `a` to `b` to a quarter of the diagonal of the model's bounds.
- `H` hides the selected object's marker and `Alt+H` shows them all (display only; the file does not change). `Delete` deletes the selected object (projection boxes and decals cannot be deleted, as they are the fill layer's projection; a ruler can be deleted, and undo brings it back).

Objects on hidden layers (including layers in a hidden group) show no marker and cannot be selected. Dropping an image from Assets on the model in the 3D view places a decal, and in Edit mode the placed decal is selected. Pressing "Edit in 3D View" in a field also selects that object. Nothing can be moved on the 2D canvas (in Edit mode it is for viewing only). One object moves at a time.

## Pose the model

When you load a model with bones from an FBX, a "Pose" tab appears in the same group as Properties (it can also be moved to a separate window).

1. Select a bone in the "Bones" tree to enter Pose mode; rings of a gizmo appear in the 3D view. Drag a ring with the left button to rotate, or click a surface to pick a bone. The mode dropdown at the left end of the options bar, the mode pie menu on `Ctrl+Tab`, View → Mode in the menu bar, and the button at the top of the Pose panel also switch the mode ("Modes" in [GUIDE_START.md](GUIDE_START.md)). Pose mode does not paint (the 2D canvas is for viewing only, too). The selected bone can also be moved with `G`, `R` and `S`, or by choosing Move, Rotate or Scale in the tool strip and dragging from the bone's surface with the left button (they work as in Edit mode; see "Edit and Pose modes" in [GUIDE_KEYS.md](GUIDE_KEYS.md)).
2. The selected bone's position, rotation and scale can be typed (rotation is Euler angles in the same order as the Unity Inspector). You can reset a field, the bone, the bone and its children, everything, or one BlendShape at a time.
3. Each pose operation is one pose undo step (`Ctrl+Z` in Pose mode). Presets and takes cannot be applied while you are painting.

A model whose FBX contains takes (animations) shows a "Takes" section. Pick a take and a frame with the frame slider, then press "Apply as Pose" to apply the bones and BlendShape weights at that time to the current pose (moving the slider alone does not apply it). You can adjust the result by hand afterwards.

"Pose Presets" saves the current pose under a name in `pose_presets/` in the settings folder. A preset holds the differences of the bones that differ from rest, by bone name path, so it applies to a model with the same bone names (BlendShapes are not included, and bones that do not fit are skipped with reasons). You can apply a preset, apply it mirrored, overwrite it with the current pose, rename it or delete it. Mirroring uses the naming `.L`/`.R`, `_L`/`_R`, `Left`/`Right` and 左/右 to move the pose to the paired bone.

"Hide Surfaces" hides the faces that a selected bone influences (faces whose three vertices average at least the "Threshold" weight on the bone; "Include Children" adds the bones under it) from the 3D view. Hidden faces are also ignored by painting, ranges and the eyedropper (the 2D canvas and saving are not affected). A way of hiding can be saved under a name in `hide_presets/`, and several can be active at once.

The pose of the project's model is saved in the `.ylp` and comes back when the same model is opened (bones and BlendShapes that do not fit are skipped with reasons).

## Bake mesh maps

The "Bake Mesh Maps…" button (flame icon) in the bar at the bottom of the Texture Sets panel (when the bar is too narrow and the button is hidden, "Bake Mesh Maps…" in the right-click menu of a set row) bakes, from the model in the 3D view and for each checked texture set, World Normal, Position, Ambient Occlusion (AO), Curvature, Thickness, Tangent Normal, Height, ID, Bent Normal and Opacity. It runs in another thread with progress and cancel.

"Bake Mesh Maps…" is also in the table under Everywhere & View in Keyboard Shortcuts as an action with no default key, so you can assign a key to it or put it in a pie menu ([GUIDE_KEYS.md](GUIDE_KEYS.md)).

Check the maps in the list on the left and press "Bake Checked Maps". The state at the right of each row (Current, Stale, Not baked, Unchecked) and the eye icon show a baked map over the 2D canvas. "Common Settings" has padding and antialiasing, and "Bake On" is Auto, GPU or CPU.

- Auto uses the GPU when a hardware GPU is available and the CPU otherwise. When the GPU is unavailable, over its budget or fails midway, the reason is shown and the bake is redone on the CPU.
- On the GPU, the maps that cast rays (AO, thickness and bent normal) are baked on the RT cores (hardware ray tracing; Vulkan on Windows, Metal on macOS) when the GPU has them. The same rays are cast both on the RT cores and by the path that does not use them (compute), and the RT cores are used only when the answers agree; otherwise, or when they are not available, the bake uses compute. The path used is shown on the last-bake line of the Bake window, and pointing at that line shows why the RT cores were not used. On a PC whose driver hangs, turn off Edit → Settings… → Display → Use RT cores for baking to always bake with compute (on by default). The environment variable `YOLUPAINTER_BAKE_RAY_QUERY=0` turns it off too.
- GPU values are not byte-identical to the CPU's (AO, thickness and bent normal differ by up to about 3%; the measurements are in [`crates/yolu-gpu/README.md`](https://github.com/YozoraKurage/YoluPainter/blob/main/crates/yolu-gpu/README.md)).
- Projection from a high-poly model is not available, so Tangent Normal, Height and Opacity are uniform.

Baked maps are saved in the `.ylp` for each set. A map whose model or settings changed is shown as "Stale" and is not used for the exported AO.

### ID maps and manual colors

The ID map paints one flat color for each "Colors from" choice (Slot, Mesh, UV Island, Mesh Part). In the properties of the ID Color Select tool (`Shift+W`), "Manual part colors" lets you set a color for each part (also by hex, and "Automatic" returns it). Manual colors become the ID map's colors and are saved in the `.ylp`. Colors made on another model can only be reset all at once.

## Work with overlapping UVs

Where two or more triangles cover the same texel (such as the two sides of a mirror laid on top of each other), a bake takes the value of just one island. When painting, both sides share the same pixels, so you cannot paint one side alone (the first stroke that hits an overlapping texel in a set shows a notice once).

- See them: turn on "Overlapping UVs" in the View menu, and the 2D canvas shows the overlapping texels in a color, with the edges of the islands involved. Change the color and opacity with the right-hand swatch in the "UV Wireframe" row of Edit → "Settings…".
- Choose which side is baked: "Priority" on the "Overlapping UVs" page of the bake window chooses whose value an overlapping texel takes. "Index" is the triangle with the lower index (the starting setting), "Area" the island with the larger 3D area, and "+X" and "−X" the island on the model's +X or −X side (one side of a mirror). You can also turn on "Skip islands outside 0–1".
- Pick islands by hand: press + for "Preferred Islands" or "Islands Not Baked", then click an island in the 2D canvas or the 3D view to add it to the list (clicking a listed island removes it; `Esc` finishes). In the map on the right, click an island to choose Prefer, Skip or Remove from its menu. The island under the pointer is highlighted in the canvas and the 3D view too.
- From Polygon Fill: right-click an island with Polygon Fill, in the 2D canvas or the 3D view, and choose "Prefer in Bake" or "Skip in Bake". In 2D, clicking again on an overlap undoes the previous fill and fills the next island.

The bake priority is saved in the `.ylp` for each texture set and is one undo step. Islands picked by hand work only on a model with the same triangle count, UVs, slots and renderers.

## Open from Unity (Live Link)

A Unity editor on the same PC can open a scene model (such as an avatar) in this app. When you open it from the Live Link window in Unity, the app reads the FBX and images, creates a texture set for each material, and applies the pose and the material values. When you paint and export, Unity imports the PNGs that were written. "Live Link" in the File menu turns acceptance on or off. How to use it is in [UNITY.md](UNITY.md), and the specification of the exchange is in [LIVELINK.md](LIVELINK.md).
