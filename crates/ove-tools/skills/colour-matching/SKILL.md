---
name: Colour matching
description: Make shots from different cameras or lighting look like they belong together, then build one look on top. Works in four passes (inspect, normalise, match, look) and checks each pass on frames. Use when the person asks to match colour, fix white balance between shots, or grade a sequence.
version: 1
---

# Colour matching

Goal: neighbouring shots look like the same place and time, and the look sits on top of that match instead of
hiding problems.

Work in this order. Do not start on the look until the shots match.

## 1. Inspect

1. Run `print_timeline` and `list_media`. Group the clips by source camera or setting (names and resolutions help).
2. Pick one hero shot per group: the best exposed, most neutral one. Ask with `ask_user` if the person has a
   favourite.
3. Run `look` with `stats: true` at the middle of each clip you will grade. Note brightness, the black share and
   the clipped share per clip. Use `grab_frame` on the hero shot's source if you need the ungraded image.

## 2. Normalise

Fix each clip on its own, with `colorGrading` on the clip:
- Exposure first: bring mid-tones near the hero shot's brightness with `exposure`. Keep the clipped share near zero.
- Then white balance: use `temperature` and `tint` until whites and skin look neutral.
- Then contrast: small `contrast`, `highlights` and `shadows` moves only.

Apply one clip per `apply_timeline` call so each step is one undo. Keep every item `id`.

## 3. Match

1. Put each clip next to its hero shot with `look` at both times.
2. Move `exposure`, `temperature` and `saturation` on the clip, not on the hero, until the two read the same.
3. Check skin tones first, then whites, then the sky or walls. Skin is what viewers notice.
4. When stats and frames agree, move on. Values within a few points of each other are enough.

## 4. Look

1. Only now add a creative look. A LUT preset (`cinematic`, `warm`, `cool`, `faded-film` and others) at an
   intensity of 0.3 to 0.6 usually works better than full strength.
2. Put the look on an adjustment item above the graded clips when it covers the whole sequence, so it can be
   changed in one place.
3. Run `look` with `stats: true` again across the sequence. The black and clipped shares should stay close to what
   they were after matching.

## 5. Finish

Run `verify_timeline`. Tell the person which shot each group was matched to, what look you used and at what
intensity, and which clips were hard to match (for example mixed light in one shot).
