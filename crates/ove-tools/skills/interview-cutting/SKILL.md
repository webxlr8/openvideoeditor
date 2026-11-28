---
name: Interview cutting
description: Tighten an interview or talking-head recording by removing dead air and filler words while keeping every sentence ending and the speaker's natural rhythm, then checking the seams. Use when the person asks to clean up, tighten, shorten or remove the ums and pauses from an interview, podcast or talk.
version: 1
---

# Interview cutting

Goal: the talk is shorter and easier to follow, it still sounds like the speaker, and no sentence loses its last
word.

## 1. Get the words

1. Run `print_timeline` and note the dialogue clips and the `rev`.
2. Run `get_transcript`. If media is listed under `untranscribed`, run `transcribe_media` for it first. If that asks
   for model consent, stop and tell the person how to give it.

## 2. Remove dead air first

1. Run `remove_silence` with `apply: false` and the default settings. Read the plan: how many cuts, how many seconds
   saved.
2. If the plan cuts more than about a third of the runtime, use `relativeDb: 15` or a longer `minPause` and plan
   again. Over-tight cuts sound breathless.
3. Apply it once the plan looks right. That is one undo step.

## 3. Then fillers

1. Run `find_fillers`. Only the candidates it selects by default are safe to cut without listening.
2. Leave the flagged context words (such as "like" or "you know") unless the person asks for an aggressive cut.
   They often carry meaning.
3. Cut with `remove_words` using the selected ids and `pause: "balanced"`. Use `apply: false` first when there are
   more than 30 cuts.

## 4. Keep sentence ends

- Never cut the last word of a sentence, even if it is quiet or trails off.
- Keep a breath before a new topic. A pause before a key point is part of the delivery.
- Do not cut inside a laugh or a word the speaker repeats for emphasis.

If a cut you planned breaks one of these rules, drop that cut.

## 5. Check the seams

1. Run `get_transcript` again and read the text around five cuts. Each should read as one natural sentence.
2. Run `look` at a few cut points to make sure no jump cut lands on an awkward frame. If the person wants smooth
   cuts, suggest covering them with B-roll or a slight zoom; do not add those unasked.
3. Run `verify_timeline`.

## 6. Report

Tell the person how much shorter the video is, how many pauses and fillers you removed, and that one Undo reverts
each pass. If something went wrong, `undo_my_changes` with `scope: "turn"` reverts this turn's cuts only.
