# アイコンの第三者表記

白・48 px の PNG。Unity 版からコピーしたものと、同じ方法で SVG を白くして描画したものを含む。
Fluent は [Microsoft Fluent UI System Icons](https://github.com/microsoft/fluentui-system-icons)、
Phosphor は [Phosphor Icons](https://github.com/phosphor-icons/core) の MIT 許諾。原文は後段に保持する。
元 SVG の取得コミットは記録されていないため、版を推定しない。確認済み PNG の SHA-256 は `tools/licenses-reviewed.json` に固定する。

`tools/` の通常版は regular、`_selected` は Fluent の filled・Phosphor の fill（長方形と楕円の選択は bold、多角形は通常版と同じ regular）。
その他は regular（`lock_filled` のみ filled）。`arrow_maximize`・`arrow_minimize`・`copy_add` はスタンドアロン側の追加。`snap_ruler`・`snap_special` は regular のアイコンに bold の磁石を重ねた物。
多角形選択の通常・選択中の 2 枚は Unity 版の `uv_wireframe.png`（Phosphor polygon regular）とバイト一致する。

| 同梱 PNG（拡張子省略） | セット | 元の名前 | 太さ |
|---|---|---|---|
| `3d_rotation` | fluent | `hexagon` | regular |
| `accessibility` | fluent | `accessibility` | regular |
| `add` | fluent | `add` | regular |
| `anchor` | phosphor | `anchor` | regular |
| `arrow_drop_down` | fluent | `chevron_down` | regular |
| `arrow_maximize` | fluent | `arrow_maximize` | regular |
| `arrow_minimize` | fluent | `arrow_minimize` | regular |
| `arrow_move` | fluent | `arrow_move` | regular |
| `auto_awesome` | fluent | `sparkle` | regular |
| `blur_on` | fluent | `blur` | regular |
| `check` | fluent | `checkmark` | regular |
| `chevron_right` | fluent | `chevron_right` | regular |
| `close` | fluent | `dismiss` | regular |
| `color_square` | fluent | `square` | regular |
| `content_copy` | fluent | `copy` | regular |
| `contrast` | fluent | `circle_half_fill` | regular |
| `conversion_path` | fluent | `pen` | regular |
| `copy_add` | fluent | `copy_add` | regular |
| `data_scatter` | fluent | `data_scatter` | regular |
| `delete` | fluent | `delete` | regular |
| `deselect` | fluent | `select_all_off` | regular |
| `document_copy` | fluent | `document_copy` | regular |
| `edit` | fluent | `edit` | regular |
| `error_circle` | fluent | `error_circle` | regular |
| `expand_less` | fluent | `chevron_up` | regular |
| `expand_more` | fluent | `chevron_down` | regular |
| `flip` | fluent | `flip_horizontal` | regular |
| `flip_vertical` | fluent | `flip_vertical` | regular |
| `folder` | fluent | `folder` | regular |
| `folder_open` | fluent | `folder_open` | regular |
| `format_color_fill` | fluent | `paint_bucket` | regular |
| `grid_dots` | fluent | `grid_dots` | regular |
| `import` | fluent | `arrow_import` | regular |
| `info` | fluent | `info` | regular |
| `ink_stroke` | fluent | `ink_stroke` | regular |
| `invert_colors` | fluent | `shape_exclude` | regular |
| `keyboard_arrow_down` | fluent | `arrow_down_right` | regular |
| `layers` | fluent | `layer` | regular |
| `library` | fluent | `library` | regular |
| `light_mode` | fluent | `weather_sunny` | regular |
| `link_off` | fluent | `link_dismiss` | regular |
| `local_fire_department` | fluent | `fire` | regular |
| `lock` | fluent | `lock_closed` | regular |
| `lock_filled` | fluent | `lock_closed` | filled |
| `lock_transparency` | fluent | `transparency_square` | regular |
| `more_horizontal` | fluent | `more_horizontal` | regular |
| `opacity` | fluent | `drop` | regular |
| `paint_brush` | fluent | `paint_brush` | regular |
| `palette` | fluent | `color` | regular |
| `play` | fluent | `play` | regular |
| `quick_mask` | fluent | `shape_organic` | regular |
| `record` | fluent | `record` | regular |
| `restart_alt` | fluent | `arrow_reset` | regular |
| `rotate_90_degrees_ccw` | fluent | `arrow_rotate_counterclockwise` | regular |
| `rotate_90_degrees_cw` | fluent | `arrow_rotate_clockwise` | regular |
| `save` | fluent | `save` | regular |
| `search` | fluent | `search` | regular |
| `select_all` | fluent | `select_all_on` | regular |
| `shape_intersect` | fluent | `shape_intersect` | regular |
| `shape_subtract` | fluent | `shape_subtract` | regular |
| `shape_union` | fluent | `shape_union` | regular |
| `shapes` | fluent | `shapes` | regular |
| `snap_ruler` | fluent + phosphor | `ruler`（regular）＋ `magnet`（bold）を重ねた物 | regular + bold |
| `snap_special` | phosphor | `compass-tool`（regular）＋ `magnet`（bold）を重ねた物 | regular + bold |
| `square` | fluent | `image` | regular |
| `stop` | fluent | `stop` | regular |
| `stylus` | fluent | `inking_tool` | regular |
| `swap_horiz` | fluent | `arrow_swap` | regular |
| `sync` | fluent | `arrow_sync` | regular |
| `target` | fluent | `target` | regular |
| `texture` | fluent | `data_area` | regular |
| `tools/brush` | phosphor | `paint-brush` | regular |
| `tools/brush_selected` | phosphor | `paint-brush` | fill |
| `tools/eraser` | fluent | `eraser` | regular |
| `tools/eraser_selected` | fluent | `eraser` | filled |
| `tools/eyedropper` | fluent | `eyedropper` | regular |
| `tools/eyedropper_selected` | fluent | `eyedropper` | filled |
| `tools/fill` | fluent | `paint_bucket` | regular |
| `tools/fill_selected` | fluent | `paint_bucket` | filled |
| `tools/gradient` | phosphor | `gradient` | regular |
| `tools/gradient_selected` | phosphor | `gradient` | fill |
| `tools/id-select` | phosphor | `swatches` | regular |
| `tools/id-select_selected` | phosphor | `swatches` | fill |
| `tools/lasso` | fluent | `lasso` | regular |
| `tools/lasso_selected` | fluent | `lasso` | filled |
| `tools/liquify` | fluent | `ink_stroke` | regular |
| `tools/liquify_selected` | fluent | `ink_stroke` | regular |
| `tools/magic-wand` | fluent | `wand` | regular |
| `tools/magic-wand_selected` | fluent | `wand` | filled |
| `tools/move` | fluent | `arrow_move` | regular |
| `tools/move_selected` | fluent | `arrow_move` | filled |
| `tools/path` | fluent | `pen` | regular |
| `tools/path_selected` | fluent | `pen` | filled |
| `tools/polygon-fill` | fluent | `triangle` | regular |
| `tools/polygon-fill_selected` | fluent | `triangle` | filled |
| `tools/ruler` | fluent | `ruler` | regular |
| `tools/ruler_selected` | fluent | `ruler` | filled |
| `tools/select-ellipse` | phosphor | `circle-dashed` | regular |
| `tools/select-ellipse_selected` | phosphor | `circle-dashed` | bold |
| `tools/select-pen` | phosphor | `highlighter` | regular |
| `tools/select-pen_selected` | phosphor | `highlighter` | fill |
| `tools/select-polygon` | phosphor | `polygon` | regular |
| `tools/select-polygon_selected` | phosphor | `polygon` | regular |
| `tools/select-rectangle` | phosphor | `selection` | regular |
| `tools/select-rectangle_selected` | phosphor | `selection` | bold |
| `tools/text` | fluent | `text_t` | regular |
| `tools/text_selected` | fluent | `text_t` | filled |
| `tune` | fluent | `options` | regular |
| `video_clip` | fluent | `video_clip` | regular |
| `view_in_ar` | fluent | `cube` | regular |
| `vignette` | fluent | `image_circle` | regular |
| `visibility` | fluent | `eye` | regular |
| `visibility_off` | fluent | `eye_off` | regular |
| `warning` | fluent | `warning` | regular |
| `window_maximize` | fluent | `maximize` | regular |
| `window_minimize` | fluent | `subtract` | regular |
| `window_restore` | fluent | `square_multiple` | regular |

`tools/liquify` and `tools/liquify_selected` reuse the existing Fluent UI `ink_stroke` PNG unchanged.

Icons added for the selection tools (same process; Fluent regular unless noted): `shape_union` (shape_union), `shape_subtract`
(shape_subtract), `shape_intersect` (shape_intersect), `edit` (edit), `quick_mask` (shape_organic), and the selection pen tool icon
`tools/select-pen` (Phosphor highlighter regular) with `tools/select-pen_selected` (Phosphor highlighter fill). The rectangle selection
tool icons `tools/select-rectangle` and `tools/select-rectangle_selected` are Phosphor selection regular and bold.

`uv_wireframe` is copied unchanged from the Unity package: Phosphor `polygon` (MIT).

`local_fire_department` is copied unchanged from the Unity package: Fluent UI `fire` regular (MIT). It marks the Bake Mesh Maps button.

`window_minimize`・`window_maximize`・`window_restore` are the standalone app's window buttons on Windows (Fluent regular; the close button is the existing `close`). They were rendered white at 48 px by the same procedure as the rest.

`error_circle` (the error mark of the Log panel and its Errors Only toggle) and `document_copy` (its Copy All button) are Fluent regular, rendered white at 48 px by the same procedure as the rest.

`record`・`stop`・`play` (the Actions panel: start and stop recording, play an action) are Fluent regular, rendered white at 48 px by the same procedure as the rest.

`more_horizontal` (the All Modes button of the selection tools' Tool Properties) is Fluent regular, rendered white at 48 px by the same procedure as the rest.

## Fluent UI System Icons

```
MIT License

Copyright (c) 2020 Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Phosphor Icons

```
MIT License

Copyright (c) 2023 Phosphor Icons

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
