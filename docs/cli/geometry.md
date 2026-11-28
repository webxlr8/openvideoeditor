# `ove geometry`

```sh
ove geometry <project> --at <s> [--height <px>] [--json]
```

**geometry** reports where every visible layer is on screen at a time: the corners of its source after placement and
transform, the corners of the part its crop keeps, and its stacking order. The values come from the same placement
the compositor draws with, so they match the rendered frame. MCP and the in-app assistant: `layer_geometry`.

- `--at <s>` is a timeline time in seconds. A time at or after the end of the timeline is refused (exit 1) with the
  timeline's length.
- `--height <px>` is the canvas height the corners are measured on. The default is the project's reference height
  (1080 for 16:9, 1920 for 9:16), whose pixels are the units of `transform` x and y.

The output (`--json`):

| Field | Meaning |
|---|---|
| `width`, `height` | The canvas the corners are measured on |
| `layers[]` | The visible layers, bottom first |
| `layers[].itemId`, `.itemType` | The item and its type (`video`, `text`, `caption`, `code`, `adjustment`, `shader`) |
| `layers[].index` | Stacking order: 0 is the bottom layer |
| `layers[].quad` | The source's top-left, top-right, bottom-right and bottom-left corners, `[x, y]` pixels from the frame's top-left corner (x right, y down). A mirrored layer's source top-left lands on the right |
| `layers[].cropQuad` | The same corners of the part the crop keeps |
| `layers[].rotation` | Clockwise degrees |
| `layers[].opacity` | 0..1 |
| `layers[].selectable` | False for adjustment and shader layers, which cover the frame and have no pixels of their own |

A caption's corners are the bounds of the words it draws at that time. A text item's corners include its padding,
outline and shadow. A layer in a transition is reported at its resting place, without the transition's motion. Items
that draw nothing at that time (an empty title, a hidden item, a placeholder) are not listed.

Examples (project `Demo`: a title from 2 s to 4 s; the timeline is 4 s long):

```sh
ove geometry Demo --at 3 --json          # exit 0
ove geometry Demo --at 3 --height 540          # exit 0
ove geometry Demo --at 60          # exit 1
```

The last one prints `error: 60 s is past the end of the timeline, which is 4 s long`.
