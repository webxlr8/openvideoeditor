# Timeline projection format (format 1)

`timeline.tsx` in a bundle is a literal TSX view of the project document, printed after every revision. People and
agents edit it and apply it back (`ove source apply`); the app merges the edits onto the live document. The file is
parsed, never executed. Behaviour contract: docs/spec/document-model.md (Projection rules, Apply / merge, Verify);
requirements: openspec/changes/add-timeline-projection/specs. Golden examples: `crates/ove-projection/tests/corpus/`.

## 1. File layout

```
/*
 * ove timeline projection, format 1
 * <two lines of guidance>
 *
 * aspect: 16:9
 * duration: 14s
 * tracks: 5 · items: 9
 *
 * media:
 *   name       id       type   duration  resolution  status
 *   interview  m_vid    video  12.5s     3840x2160   awaiting-probe
 * cast: Ana (host), Ben
 * diagnostics: (none)            | or one indented line per warning: "warning L:C code: message"
 * pending: 1
 * rev: 1762000005000
 */
export default (
  <timeline aspect="16:9">
    <track id="t_main" name="Main" kind="video">
      <clip id="i_vid" name="Interview" start={0} end={8} in={1.5} src="interview" volume={80} />
    </track>
  </timeline>
);
```

- LF line endings, UTF-8, two-space indentation, a trailing newline. Same document → byte-identical file on every OS.
- Exactly one statement: `export default ( <timeline> … </timeline> );`. Anything else is `structure.root`.

## 2. Header

A leading block comment. It is informational: edits to it are ignored on apply, **except `rev`**, which names the
revision the file was printed from (the merge base). Lines, in order: format marker, guidance, `aspect`,
`duration` (latest item end), `tracks · items` counts, the media table, `cast`, `diagnostics`, `pending` count, `rev`.

- **Media table:** one row per media item, sorted by id. `name` is the file stem of `data.fileName`, de-duplicated in
  id order with `-2`, `-3`, … (and never equal to another media id); `status` is the registry probe status, or
  `offline` when the file is missing.
- **Diagnostics:** warnings from the last `source apply` (e.g. unknown keys), until the next apply.

## 3. Elements and attributes

| Element | Contains | Attributes |
|---|---|---|
| `<timeline>` | `<track>` only | `aspect` (16:9, 4:3, 3:4, 9:16, 1:1, 4:5); others warn `attr.unknown` and are ignored |
| `<track>` | items only | `id` (omit for a new track), `name`, `kind` (= document `type`: video, audio, caption, adjustment, layout), `locked`, `visible`, `muted`, unknown fields |
| `<clip>` (video), `<image>`, `<audio>`, `<text>`, `<caption>`, `<code>`, `<adjustment>`, `<shader>` | nothing (self-closing) | see below |

Track order is element order (first = top layer); the document's `order` is never written. An item's track is the
`<track>` it is inside; its type is its tag. Within a track, items print by start time, then id.

Item attribute renames: `start`↔`startTime`, `end`↔`endTime`, `in`↔`trimStart`, `out`↔`trimEnd`,
`src`↔`mediaItemId` (printed as the media name). Writing a document name (`startTime`, `trackId`, `type`, …) is
`attr.reserved`.

**Attribute order** (printer): `id`, `name`, `start`, `end`, `in`, `out`, `src`, `pending`, `pendingDescription`, then
content fields of the type, look fields (transform, crop, opacity, …, effects), grade, sound fields — the order of the
field registry (`docs/spec/project-schema-v1.md`, per type) — then unknown fields sorted by key.

**Default omission:** fields equal to their default are not printed (e.g. `opacity={1}`, `volume={100}`,
`blendMode="normal"`); inside known objects, keys at their default are dropped and an object left empty is omitted.
Absent on apply means "default".

**Unknown fields** print as attributes when the name is a valid JSX attribute name (`[A-Za-z_$][A-Za-z0-9_$-]*`)
and not reserved (`id start end in out src trackId type startTime endTime trimStart trimEnd mediaItemId extraFields`
for items; `id name kind order type extraFields` for tracks). Others are grouped in one attribute:
`extraFields={{"z weird key": 1}}`. Unknown attributes round-trip unchanged without warnings.

## 4. Values

Attribute values are literals only: strings, numbers, booleans, `null`, arrays and objects of literals. A leading
`-`/`+` on a number is allowed. Expressions, identifiers, calls, spreads, computed keys, shorthand properties and
template literals with substitutions are `literal.only` errors. Comments are dropped. Boolean shorthand (`muted`)
means `true`.

- **Strings:** printed quoted (`name="Intro"`) unless they contain `"`, `\` or a control character; then as a braced
  JSON string (`textContent={"C:\\new\nline"}`). Quoted attribute text is taken literally: no escapes, no
  character references (`textContent="C:\new"` stores backslash + `n`).
- **Numbers:** plain decimals, never exponent notation. Times are seconds at microsecond precision printed with the
  fewest digits (`2`, `2.5`, `0.000001`). Integral values print as integers.
- **Objects:** `{{key: value, …}}` with bare keys when they are identifiers, otherwise JSON-quoted keys; inside
  objects and arrays strings are JSON strings.
- **Layout:** an element prints on one line when it fits in 120 columns, otherwise one attribute per line with the
  closing `/>` (or `>`) on its own line. A single long value is never split.

## 5. `src`

Resolution order: media name from the base revision's header table, then the current one; media id; file path —
absolute, `./`, `../`, `~/`, or relative to the bundle. A path already in the registry (compared canonically,
case-insensitively on macOS and Windows) reuses that media item. A new existing file is registered on apply as a
media item awaiting probe, with a stable id derived from its path (so applying the same file twice registers it once);
the next projection shows its name. A path that does not exist is `src.unresolved`.

## 6. Pending placeholders

`pending={true}` with an optional `pendingDescription`; start/end give its duration. Counted in the header and by
verify (error). Filling = setting `src` and removing `pending`: the id, position and duration stay unless edited.

## 7. Apply semantics (summary)

1. Parse → diagnostics. Errors refuse the whole apply; warnings apply and are listed in the next header.
2. The base is the revision named by `rev:`. Missing/malformed → `rev.missing`/`rev.malformed`; newer than the
   project → `rev.newer`; no longer retained → `rev.pruned`.
3. Items with an `id` present in the base are edits of that item; items without `id`, or with an id not in the base,
   are new (they keep a given id if it is free, else get a new one); base items missing from the file are removed
   ("removed N items" with ids and names); a changed id is a removal plus an addition.
4. Field-level 3-way merge (known objects one key deeper, arrays whole): base B, file F, current C. C = B → F;
   C = F → nothing; otherwise a conflict, file wins by default, `--prefer editor` keeps C; every conflict is reported
   with id, field, both values and the side kept. Removal vs edit is a conflict resolved the same way. Track order and
   aspect are single values under the same rule.
5. A new item identical to one the editor gained since the base (the same file applied twice) is already applied.
6. The result is one batch through the document pipeline (invariants enforced, one revision, one undo step); an empty
   change set creates no revision.

Text-edit apply takes `{oldText, newText}` edits against the projection printed at the current revision: each
`oldText` must occur exactly once (`edit.ambiguous` lists every match), edits must not overlap (`edit.overlap`),
`newText: ""` deletes, insertion anchors on existing text; the result is applied as above with that revision as base.

## 8. Diagnostic codes

| Code | Severity | Meaning |
|---|---|---|
| `syntax` | error | Not valid TSX (after an error inside an item, that item is skipped and checking continues) |
| `structure.root` | error | Not exactly `export default (<timeline>…</timeline>);` |
| `structure.element` | error | Unknown tag, item outside a track, element inside an item, fragment |
| `structure.text` | error | Text between elements |
| `structure.attribute` | error | Namespaced attribute (`a:b`) |
| `literal.only` | error | A non-literal value |
| `attr.duplicate` | error | Attribute (or object key) given twice |
| `attr.required` | error | Item without `start`/`end`, track without `kind` |
| `attr.reserved` | error | Document field name used instead of its projection form |
| `attr.unknown` | warning | Unknown `<timeline>` attribute (ignored) |
| `enum.known`, `value.type`, `value.range`, `value.nonNegative`, `value.finite` | error | Value invalid for a known field (field registry) |
| `object.key.unknown` | warning | Unknown key inside a known object (kept) |
| `src.unresolved` | error | `src` is not a media name or id and no file exists at the path |
| `item.id.unique`, `track.id.unique` | error | An id appears twice in the file |
| `item.time.order` | error | `end` is not after `start` |
| any document invariant code | error | The merged result would violate a document invariant (reported at the item) |

## 9. Verify lints (this change)

| Lint | Severity | Finding |
|---|---|---|
| `overlap` | error | Two items on one track overlap in time |
| `gap` | warning | Uncovered time on the main track — the bottom-most video track, the base layer — between 0 and its last item (≥ 1 ms) |
| `past-source` | warning | A clip/audio item uses more source than the media has (`in + (end − start) × speed > duration`): video holds its last frame |
| `missing-media` | error | Referenced media is not in the project, offline, or failed to probe |
| `pending` | error | A pending placeholder |

Verify exits 1 when any finding is an error. Later changes register more lints (off-screen text, invalid
compositions) in the same registry.

## 10. Typings

`timeline.d.ts` is generated from the field registry (committed copy: `docs/spec/timeline.d.ts`, CI drift check) and
written next to `timeline.tsx`. It declares `JSX.IntrinsicElements` for every tag. Attribute and object types include
`[key: string]: unknown` because `check` accepts unknown attributes and keys; `tools/check-projection-types.sh`
type-checks the golden corpus against it with `tsc --strict`.
