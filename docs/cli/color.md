# `ove color`

```sh
ove color inspect <project> [--at <s>] [--item <id>] [--isolate] [--reference <item|media>] [--reference-at <s>] [--strength <0-1>] [--height <px>] [--backend auto|cpu|gpu]
```

**inspect** measures the colour of the frame that export writes at a time. The time is snapped to the export frame
grid (the frame that starts at or before it). The frame is rendered by the same compositor as `look` and export, at
`--height` lines (default 1080, the export default), on the same compositing backend as export (`--backend`, see
`ove docs export`). MCP: `inspect_color`.

- `--item <id>` names the shot. Without `--at`, the time is the middle of the item.
- `--isolate` measures the item alone: layers above and below it (titles, overlays, adjustment layers) are left
  out, and so is the area the item does not cover.
- `--reference <item|media>` compares the shot with a reference: an item (rendered alone, at `--reference-at` on the
  timeline, default its middle) or a media file (at `--reference-at` seconds of the source, default 0).

The readout (`stats`):

| Field | Meaning |
|---|---|
| `luma.mean`, `luma.p2`, `luma.p98` | BT.709 luma in percent of full scale: mean, 2nd and 98th percentile |
| `luma.histogram` | 16 equal luma bins, percent of pixels in each |
| `clipping.low`, `clipping.high` | percent of pixels with every channel at 2/255 or below, or any channel at 253/255 or above |
| `zones.shadows`, `.midtones`, `.highlights` | pixels below 33.3 %, between, and from 66.7 % luma: `share` (percent), mean `rgb` (0..1) and mean `lab` (CIELAB, D65) |
| `balance.a`, `balance.b` | mean CIELAB a* (negative green, positive magenta) and b* (negative blue, positive yellow) |
| `saturation` | mean HSV saturation, 0..1 |
| `hue` | 12 bins of 30° centred on 0° (red), 30°, 60° (yellow) … 330°: percent of the total saturation |
| `colorful` | percent of pixels with saturation ≥ 0.25 and a brightest channel ≥ 15 % |
| `skin.share`, `skin.deviation` | percent of pixels within 30° of the skin line (123° on a BT.709 vectorscope), and their mean angle from it in degrees (positive toward yellow, negative toward red and magenta; none when below 0.5 % of pixels) |

With `--reference`, the output adds `reference` (what was measured, with its `stats`), `gap` and a suggestion:

- `gap.shadows`, `.midtones`, `.highlights`: CIEDE2000 between the zones' mean colours (none when a zone has under
  0.5 % of the pixels in either frame). `gap.mean` weights them by the reference's zone shares. `gap.luma`, `gap.a`
  and `gap.b` are reference minus shot.
- `suggestedOps`: one `updateItem` document op that sets the shot's `colorGrade`: its gain and offset wheels composed
  with a correction fitted per channel through matching quantiles of the two frames, and its saturation scaled to the
  reference's chroma. Lift, gamma, curves and LUTs stay. Each step is capped (gain ±0.35, offset ±0.25, saturation
  ×0.75 to ×1.33) and scaled by `--strength` (default 1).
- `suggestedEdit`: the same change as an `apply_timeline` edit (`rev` and `edits` with the item's printed element
  before and after).

The shot that a suggestion grades is `--item`, else the topmost video or image item at that time. To match a shot,
apply the suggestion, inspect again, and repeat until `gap.mean` is below 2 (usually one or two rounds).

The output is deterministic: the same request on an unchanged project prints the same bytes.

## Chroma key

The `chroma-key` effect removes a key colour from a clip (green or blue screen). Add it to the clip's `effects`:

```tsx
<video id="i_talk" start={0} end={12} src="talk" effects={[{id: "k", type: "chroma-key", keyColor: [0.12, 0.72, 0.22]}]} />
```

- `keyColor` (RGB 0..1): the screen colour. Sample it from the clip: in the viewer, click or drag on the screen.
- `tolerance` (0.35) and `softness` (0.25): colours closer to the key than `tolerance` are removed; the matte
  rises to opaque over the next `softness`. Distance is measured in hue and saturation relative to brightness, so a
  dark or unevenly lit screen keys like a bright one.
- `spill` (1): removes the screen colour reflected onto the subject near the key colour, keeping brightness.
- `choke` (−1..1, 0): positive shrinks the matte, negative grows it. `edgeBlur` (pixels, 1) softens its edge.
- `matteBlack` and `matteWhite` (0 and 1): matte levels; values below black become transparent, above white
  opaque.
- `matteView`: show the matte (white kept, black removed) instead of the result. `ove look --view matte` shows
  every key's matte without changing the project.
- A key colour that is not in the clip leaves it unchanged. Any numeric parameter takes keyframes
  (`effects.k.tolerance`).

Refusals (exit 1):

```sh
ove color inspect Demo --at 60          # exit 1
ove color inspect Demo --at 1 --reference m_missing          # exit 1
ove color inspect Demo --at 1 --isolate          # exit 1
```

- `time.outOfRange`: the time is at or after the end of the timeline; the message gives the duration.
- `reference.notFound`: the reference is not an item or media in the project; the message names it.
- `argument.missing`: `--isolate` without `--item`, or neither `--at` nor `--item`.
- `item.notFound`, `item.notAtTime` (an isolated item that is not on the timeline at that time), `item.empty` (it
  shows nothing there).
