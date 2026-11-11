# Project schema v1

Generated from the `ove-core` field registry (`cargo run -p ove-core --bin ove-schema`); CI fails if it drifts.
Behaviour contract: docs/spec/document-model.md. Machine-readable form: `docs/spec/project.schema.json`.

Conventions:
- Times are seconds as JSON numbers, stored at microsecond precision (integers when whole).
- Fields equal to their default may be omitted; projections always omit them.
- Unknown fields are preserved on read and write, in their original order. Unknown keys inside known closed objects
  are kept and reported as warnings; open objects (`{…}`) take preset parameters.
- Invariant codes are listed in `crates/ove-core/src/invariants.rs`.

## Document

| field | type | meaning |
|---|---|---|
| `format` | `"ove-project"` | format id |
| `version` | integer | schema version (1) |
| `project` | object | see Project |
| `media` | array of registry entries | where media files live and their last known state |

## Project

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `id` | string |  | yes | Project id. |
| `name` | string |  | yes | Project name (free text; the bundle folder never changes on rename). |
| `createdAt` | integer |  |  | ms since the epoch. |
| `updatedAt` | integer |  |  | ms since the epoch. |
| `stateUpdatedAt` | integer ≥ 0 |  | yes | Current revision (rev), ms-based. |
| `aspectRatio` | `16:9` \| `4:3` \| `3:4` \| `9:16` \| `1:1` \| `4:5` |  | yes | Frame aspect ratio. |
| `media` | any |  |  | Media items. |
| `timelineItems` | any |  |  | Items. |
| `tracks` | any |  |  | Tracks. |

Other project fields (`cast`, `temp`, `mediaBaseDirectory`, …) are preserved and changed with `updateProject`.

## Track

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `id` | string |  | yes | Unique track id. |
| `name` | string |  |  | Display name. |
| `order` | integer ≥ 0 |  | yes | Position; 0 is the top layer. Unique and contiguous. |
| `type` | `video` \| `audio` \| `caption` \| `adjustment` \| `layout` |  | yes | Track type. |
| `locked` | boolean | `false` |  | Locked tracks cannot be edited in the GUI. |
| `visible` | boolean | `true` |  | Hidden tracks render nothing. |
| `muted` | boolean | `false` |  | Muted tracks play no audio. |

## Media item (`project.media[]`)

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `id` | string |  | yes | Unique media id. |
| `createdAt` | integer |  |  | ms since the epoch. |
| `addedAt` | integer |  |  | ms since the epoch. |
| `data` | any |  |  | type, fileName, format, size, duration, width, height, frameRate, hasAudio, statuses, … |

## Registry entry (`media[]`)

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `mediaId` | string |  | yes | Id of the media item. |
| `nativePath` | string |  | yes | Absolute path of the file, or relative to the bundle when owned. |
| `ownedByBundle` | boolean |  | yes | The file lives in the bundle's media/ folder. |
| `probeStatus` | `awaiting-probe` \| `probed` \| `failed` | `"awaiting-probe"` |  | Probe state. |
| `size` | integer ≥ 0 |  |  | Last known file size in bytes. |
| `modifiedAt` | integer |  |  | Last known modification time, ms since the epoch. |

## Items

Every item has these head fields:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `id` | string |  | yes | Unique item id. |
| `trackId` | string |  | yes | Id of the track the item is on. |
| `type` | `video` \| `image` \| `audio` \| `text` \| `caption` \| `code` \| `adjustment` \| `shader` |  | yes | Item type. |
| `startTime` | seconds ≥ 0 |  | yes | Timeline start, seconds. |
| `endTime` | seconds ≥ 0 |  | yes | Timeline end, seconds (greater than startTime). |

### `video`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |
| `colorGrading` | object {exposure, brightness, contrast, highlights, shadows, temperature, tint, saturation, vibrance, gamma} |  |  | Basic colour adjustments. |
| `colorGrade` | object {wheels, curves, saturation, lut, match} |  |  | Wheels, curves, LUT and match. |
| `volume` | number [0..200] | `100` |  | Volume in percent. |
| `muted` | boolean | `false` |  | Muted. |
| `audioFadeIn` | object {duration, easing} |  |  | Audio fade at the start. |
| `audioFadeOut` | object {duration, easing} |  |  | Audio fade at the end. |
| `playbackSpeed` | number [0.01..100] | `1` |  | Playback speed factor. |
| `playbackDirection` | `forward` \| `reverse` | `"forward"` |  | Playback direction. |

### `image`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |
| `colorGrading` | object {exposure, brightness, contrast, highlights, shadows, temperature, tint, saturation, vibrance, gamma} |  |  | Basic colour adjustments. |
| `colorGrade` | object {wheels, curves, saturation, lut, match} |  |  | Wheels, curves, LUT and match. |

### `audio`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |
| `volume` | number [0..200] | `100` |  | Volume in percent. |
| `muted` | boolean | `false` |  | Muted. |
| `audioFadeIn` | object {duration, easing} |  |  | Audio fade at the start. |
| `audioFadeOut` | object {duration, easing} |  |  | Audio fade at the end. |
| `playbackSpeed` | number [0.01..100] | `1` |  | Playback speed factor. |
| `playbackDirection` | `forward` \| `reverse` | `"forward"` |  | Playback direction. |

### `text`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `textContent` | string |  |  | The text. |
| `fontFamily` | string |  |  | Font family. |
| `fontSize` | number [0..] |  |  | Font size in pixels. |
| `fontWeight` | any |  |  | Font weight (number or keyword). |
| `fontStyle` | `normal` \| `italic` | `"normal"` |  | Font style. |
| `color` | string |  |  | Text colour. |
| `textAlign` | `left` \| `center` \| `right` \| `justify` | `"center"` |  | Alignment. |
| `lineHeight` | number [0..] |  |  | Line height multiplier. |
| `letterSpacing` | number |  |  | Letter spacing in pixels. |
| `strokeColor` | string |  |  | Outline colour. |
| `strokeWidth` | number [0..] |  |  | Outline width in pixels. |
| `shadowColor` | string |  |  | Shadow colour. |
| `shadowBlur` | number [0..] |  |  | Shadow blur in pixels. |
| `shadowOffsetX` | number |  |  | Shadow x offset in pixels. |
| `shadowOffsetY` | number |  |  | Shadow y offset in pixels. |
| `backgroundColor` | string |  |  | Background box colour. |
| `backgroundOpacity` | number [0..1] |  |  | Background box opacity. |
| `backgroundPadding` | number [0..] |  |  | Background box padding in pixels. |
| `textAnimation` | object {preset, …} |  |  | Text animation preset and parameters. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |

### `caption`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `captionText` | string |  |  | Full caption text. |
| `captionSegments` | array of object {id, text, startTime, endTime, wordTimings} |  |  | Timed segments with word timings. |
| `captionPresetId` | string |  |  | Caption style preset. |
| `captionPosition` | any |  |  | Caption position. |
| `captionFontSize` | number [0..] |  |  | Font size in pixels. |
| `captionFontFamily` | string |  |  | Font family. |
| `captionTextColor` | string |  |  | Text colour. |
| `captionHighlightColor` | string |  |  | Active-word highlight colour. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |

### `code`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `codeComposition` | object {source, template, props, width, height, assets} |  |  | Motion-graphics composition (pure JSX of {time, width, height, assets}). |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |

### `adjustment`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `adjustmentKind` | string |  |  | Kind of adjustment layer. |
| `lutPresetId` | string |  |  | LUT preset id. |
| `lutIntensity` | number [0..1] | `1` |  | LUT blend amount. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |
| `colorGrading` | object {exposure, brightness, contrast, highlights, shadows, temperature, tint, saturation, vibrance, gamma} |  |  | Basic colour adjustments. |
| `colorGrade` | object {wheels, curves, saturation, lut, match} |  |  | Wheels, curves, LUT and match. |

### `shader`

Fields in projection attribute order:

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `name` | string | `""` |  | Display name. |
| `trimStart` | seconds ≥ 0 | `0` |  | Seconds trimmed from the start of the source. |
| `trimEnd` | seconds ≥ 0 | `0` |  | Seconds trimmed from the end of the source. |
| `mediaItemId` | string |  |  | Id of the media item the item plays. |
| `pending` | boolean | `false` |  | Placeholder awaiting generated or supplied media. |
| `pendingDescription` | string |  |  | What the pending placeholder should become. |
| `shaderComposition` | object {source, template, params, controls, amount, fadeIn, fadeOut, region, defaults} |  |  | Shader layer definition. |
| `transform` | object {x, y, scale, rotation, flipX} |  |  | Position, scale, rotation, flip. |
| `crop` | object {top, right, bottom, left} |  |  | Crop in percent per edge. |
| `opacity` | number [0..1] | `1` |  | Opacity. |
| `hidden` | boolean | `false` |  | Hidden items render nothing. |
| `zIndex` | number | `0` |  | Stacking order within the track. |
| `blendMode` | `normal` \| `multiply` \| `screen` \| `overlay` \| `darken` \| `lighten` \| `color-dodge` \| `color-burn` \| `hard-light` \| `soft-light` \| `difference` \| `exclusion` \| `hue` \| `saturation` \| `color` \| `luminosity` | `"normal"` |  | Blend mode. |
| `borderRadius` | number [0..] | `0` |  | Corner radius in pixels. |
| `gaussianBlur` | number [0..] | `0` |  | Blur radius in pixels. |
| `keyframeTracks` | array of object {property, keyframes, easing} |  |  | Animated properties. |
| `transitionIn` | object {type, presetId, duration, easing, …} |  |  | Transition at the start. |
| `transitionOut` | object {type, presetId, duration, easing, …} |  |  | Transition at the end. |
| `effects` | array of object {id, type, …} |  |  | Effect stack. |
| `captionComposition` | object {mode, fallback} |  |  | Caption depth compositing. |
| `autoReframe` | boolean | `true` |  | Fill the frame height with a centred crop when not transformed. |

### Nested objects

#### `transform`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `x` | number | `0` |  | Horizontal offset in pixels from the frame centre. |
| `y` | number | `0` |  | Vertical offset in pixels from the frame centre. |
| `scale` | number [0..] | `1` |  | Uniform scale factor. |
| `rotation` | number | `0` |  | Rotation in degrees, clockwise. |
| `flipX` | boolean | `false` |  | Mirror horizontally. |

#### `crop`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `top` | number [0..100] | `0` |  | Percent of height removed from the top. |
| `right` | number [0..100] | `0` |  | Percent of width removed from the right. |
| `bottom` | number [0..100] | `0` |  | Percent of height removed from the bottom. |
| `left` | number [0..100] | `0` |  | Percent of width removed from the left. |

#### `keyframeTracks`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `property` | `x` \| `y` \| `scale` \| `rotation` \| `opacity` \| `gaussianBlur` \| `borderRadius` |  |  | Animated property. |
| `keyframes` | array of object {time, value} |  |  | Keyframes in time order. |
| `easing` | `linear` \| `ease-in` \| `ease-out` \| `ease-in-out` | `"linear"` |  | Interpolation between keyframes. |

#### `keyframes`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `time` | seconds ≥ 0 |  |  | Seconds from the item's start. |
| `value` | number |  |  | Property value at that time. |

#### `transitionIn`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `type` | string |  |  | Built-in transition type. |
| `presetId` | string |  |  | Transition preset id. |
| `duration` | seconds ≥ 0 |  |  | Seconds. |
| `easing` | `linear` \| `ease-in` \| `ease-out` \| `ease-in-out` |  |  | Easing curve. |

#### `transitionOut`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `type` | string |  |  | Built-in transition type. |
| `presetId` | string |  |  | Transition preset id. |
| `duration` | seconds ≥ 0 |  |  | Seconds. |
| `easing` | `linear` \| `ease-in` \| `ease-out` \| `ease-in-out` |  |  | Easing curve. |

#### `effects`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `id` | string |  |  | Effect instance id. |
| `type` | string |  |  | Effect preset type. |

#### `captionComposition`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `mode` | `front` \| `behind-subject` | `"front"` |  | Draw in front of or behind the subject. |
| `fallback` | `front` \| `hide` | `"front"` |  | When no subject matte is available. |

#### `colorGrading`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `exposure` | number | `0` |  | Exposure adjustment. |
| `brightness` | number | `0` |  | Brightness adjustment. |
| `contrast` | number | `0` |  | Contrast adjustment. |
| `highlights` | number | `0` |  | Highlights adjustment. |
| `shadows` | number | `0` |  | Shadows adjustment. |
| `temperature` | number | `0` |  | White-balance temperature shift. |
| `tint` | number | `0` |  | White-balance tint shift. |
| `saturation` | number | `0` |  | Saturation adjustment. |
| `vibrance` | number | `0` |  | Vibrance adjustment. |
| `gamma` | number | `0` |  | Gamma adjustment. |

#### `colorGrade`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `wheels` | any |  |  | Lift/gamma/gain/offset wheels, each {r,g,b,y}. |
| `curves` | any |  |  | master, red, green, blue, hueVsSat, hueVsHue, hueVsLum, lumVsSat curves. |
| `saturation` | number |  |  | Global saturation. |
| `lut` | object {preset, file, intensity, stage} |  |  | LUT applied in the grade. |
| `match` | any |  |  | Reference match; fitted on apply and replaced by concrete values. |

#### `lut`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `preset` | string |  |  | LUT preset id. |
| `file` | string |  |  | Path to a .cube file (copied into the bundle on apply). |
| `intensity` | number [0..1] | `1` |  | Blend amount. |
| `stage` | `input` \| `look` | `"look"` |  | Where in the grade the LUT applies. |

#### `audioFadeIn`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `duration` | seconds ≥ 0 |  |  | Seconds. |
| `easing` | `linear` \| `ease-in` \| `ease-out` \| `ease-in-out` | `"linear"` |  | Fade curve. |

#### `audioFadeOut`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `duration` | seconds ≥ 0 |  |  | Seconds. |
| `easing` | `linear` \| `ease-in` \| `ease-out` \| `ease-in-out` | `"linear"` |  | Fade curve. |

#### `textAnimation`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `preset` | string |  |  | Text animation preset id. |

#### `captionSegments`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `id` | string |  |  | Segment id. |
| `text` | string |  |  | Segment text. |
| `startTime` | seconds ≥ 0 |  |  | Seconds, item-relative. |
| `endTime` | seconds ≥ 0 |  |  | Seconds, item-relative. |
| `wordTimings` | array of object {word, start, end} |  |  | Per-word timings. |

#### `wordTimings`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `word` | string |  |  | The word. |
| `start` | seconds ≥ 0 |  |  | Seconds. |
| `end` | seconds ≥ 0 |  |  | Seconds. |

#### `codeComposition`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `source` | string |  |  | JSX source: a pure function of {time, width, height, assets}. |
| `template` | string |  |  | Template id (instead of source). |
| `props` | any |  |  | Template props. |
| `width` | number [1..] |  |  | Composition width in pixels. |
| `height` | number [1..] |  |  | Composition height in pixels. |
| `assets` | any |  |  | Named assets: name → src or {src, in, loop}. |

#### `shaderComposition`

| field | type / range | default | required | meaning |
|---|---|---|---|---|
| `source` | string |  |  | Shader source. |
| `template` | string |  |  | Shader template id (instead of source). |
| `params` | any |  |  | Parameter values. |
| `controls` | any |  |  | Control definitions {label, min, max, step, format, toggle}. |
| `amount` | number |  |  | Overall amount. |
| `fadeIn` | seconds ≥ 0 |  |  | Seconds. |
| `fadeOut` | seconds ≥ 0 |  |  | Seconds. |
| `region` | `all` \| `subject` \| `background` |  |  | Where the shader applies. |
| `defaults` | any |  |  | Template defaults. |

