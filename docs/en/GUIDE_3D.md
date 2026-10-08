# 3D View and Models

[日本語](../GUIDE_3D.md)

This page covers viewing a model in the 3D view, moving around it, and painting on its surface. Opening an FBX, posing, baking mesh maps, overlapping UVs and opening from Unity are here too.

## Move around the 3D view

Rotate with a right drag. An `Alt`+left drag also rotates, and when the direction comes within 15° of an axis view (front, back, right, left, top, bottom) it snaps to that view. While the right button is held, `W` / `S` move the view forward and back, `A` / `D` left and right, and `Q` / `E` down and up (`Shift` for faster). Pan with a middle drag or `Space`+left drag. Zoom with the wheel or `Ctrl+Space`+left drag. Releasing the right button without moving it picks the value of that face as the paint color (eyedropper; with Polygon Fill it opens the island menu). `.` over the 3D view fits the faces of the current texture set, and "Fit the whole model in view" in the top right fits the whole model. All the keys are in [GUIDE_KEYS.md](GUIDE_KEYS.md).

## Open a model

While there is no model, the 3D view shows only "Load Test Cube". Open an FBX with View → "Open an FBX in the 3D View…", "Open FBX" in the Pose panel, or by dropping a .fbx onto the window. To open from Unity, see Live Link at the end of this page.

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

On the Material tab of Properties, "Look" chooses "Standard (PBR)" or "lilToon" for each texture set. lilToon draws a reproduction of the lilToon look, and its color, normal map, reflection and advanced values can be edited in the fields. "lilToon Template" creates and assigns the user channels for the masks of the features you turned on.

The material values received from Unity appear as "Unity Values", and "Match Unity" drops the values changed here and draws with Unity's. A shader with only a similar name is not drawn as lilToon ([PREVIEW.md](../PREVIEW.md)).

## Paint on the model

The tools that work in the 3D view are Brush, Eraser, Eyedropper, Fill, Polygon Fill, ID Color Select and Path (paths are in [GUIDE_PATHS.md](GUIDE_PATHS.md)). Selections, Move / Transform, Liquify, Shape, Ruler, Gradient and Text work only on the 2D canvas.

Brush settings work in the 3D view with the same formulas as on the 2D canvas: Size, Hardness, Flow, Opacity, pressure, image tips (angle, roundness, Flip X and Flip Y, Follow direction), the stroke (Stabilizer, Curve), Taper & Pen (fade, tilt, rotation, speed), jitter, texture, dual brush, color, Color Dynamics (including Apply per tip), color mixing, erasing, stencils and the effect brushes (Blur, Smudge, Clone).

- The tip shape is laid out on the screen and projected from the camera onto the surface (it is not laid flat along the surface).
- Dabs are placed every spacing along the stroke length, as in 2D. However densely or sparsely the input points arrive, the stroke's density and the length of a fade stay the same. The spacing comes from the brush size converted to the screen at the depth of the surface where each stretch of the stroke starts.
- Stabilizer, taper and speed lengths and speeds are measured in screen points in the 3D view (in document pixels on the 2D canvas).
- The texture is read at the document pixel being painted, so its grain lines up between what you paint in 2D and in 3D.
- With 3D symmetry, a mirror copy flips the tip left to right and a radial copy rotates it.

While the 3D view is shown, the brush properties gain a "3D" group.

| Field | What it does |
|---|---|
| Paint hidden areas | Also paints surfaces hidden behind nearer ones under the brush |
| Paint back faces | Also paints faces that point away from the camera |
| Fade by angle | Paints more lightly where the surface turns away from the view ("Fade start", "Fade end") |
| Seam bleed | How far to paint outside the edges of UV islands so seams do not show |

3D Smudge and Clone sample across UV island seams. Set the clone source with an `Alt`-click released without moving (if you move, it is the snap orbit). "Aligned" keeps the offset from the previous stroke, and "All layers" reads the visible layers together.

## Pose the model

When you load a model with bones from an FBX, a "Pose" tab appears in the same group as Properties (it can also be moved to a separate window).

1. Select a bone in the "Bones" tree to enter pose mode; rings of a gizmo appear in the 3D view. Drag a ring with the left button to rotate, or click a surface to pick a bone. View → "Pose Mode" also switches the mode.
2. The selected bone's position, rotation and scale can be typed (rotation is Euler angles in the same order as the Unity Inspector). You can reset a field, the bone, the bone and its children, everything, or one BlendShape at a time.
3. Each pose operation is one pose undo step (`Ctrl+Z` in pose mode). Presets and takes cannot be applied while you are painting.

A model whose FBX contains takes (animations) shows a "Takes" section. Pick a take and a frame with the frame slider, then press "Apply as Pose" to apply the bones and BlendShape weights at that time to the current pose (moving the slider alone does not apply it). You can adjust the result by hand afterwards.

"Pose Presets" saves the current pose under a name in `pose_presets/` in the settings folder. A preset holds the differences of the bones that differ from rest, by bone name path, so it applies to a model with the same bone names (BlendShapes are not included, and bones that do not fit are skipped with reasons). You can apply a preset, apply it mirrored, overwrite it with the current pose, rename it or delete it. Mirroring uses the naming `.L`/`.R`, `_L`/`_R`, `Left`/`Right` and 左/右 to move the pose to the paired bone.

"Hide Surfaces" hides the faces that a selected bone influences (faces whose three vertices average at least the "Threshold" weight on the bone; "Include Children" adds the bones under it) from the 3D view. Hidden faces are also ignored by painting, ranges and the eyedropper (the 2D canvas and saving are not affected). A way of hiding can be saved under a name in `hide_presets/`, and several can be active at once.

The pose of the project's model is saved in the `.ylp` and comes back when the same model is opened (bones and BlendShapes that do not fit are skipped with reasons).

## Bake mesh maps

View → "Bake Mesh Maps…" bakes, from the model in the 3D view and for each checked texture set, World Normal, Position, Ambient Occlusion (AO), Curvature, Thickness, Tangent Normal, Height, ID, Bent Normal and Opacity. It runs in another thread with progress and cancel.

Check the maps in the list on the left and press "Bake Checked Maps". The state at the right of each row (Current, Stale, Not baked, Unchecked) and the eye icon show a baked map over the 2D canvas. "Common Settings" has padding and antialiasing, and "Bake On" is Auto, GPU or CPU.

- Auto uses the GPU when a hardware GPU is available and the CPU otherwise. When the GPU is unavailable, over its budget or fails midway, the reason is shown and the bake is redone on the CPU.
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
