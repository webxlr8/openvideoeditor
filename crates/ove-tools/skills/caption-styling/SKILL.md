---
name: Caption styling
description: Give the captions of a video one consistent look by picking one of the editor's 21 caption styles to suit the video's shape and tone, then checking the result on real frames. Use when the person asks to style, restyle, tidy or fix the look of captions or subtitles.
version: 1
---

# Caption styling

Goal: every caption on the timeline uses one style that reads well on this video, and nothing covers a face or the
lower-third graphics.

## 1. Read before you change anything

1. Run `print_timeline`. Note the aspect ratio in the header and every `<caption>` item with its `captionPresetId`.
2. If there are no caption items, stop and tell the person. Styling needs captions to exist first.
3. Run `look` with `count: 4` and `grid: true` to see where faces, titles and logos sit in the frame.

## 2. Pick one style

Choose by the video's shape first, then by its tone. Ask with `ask_user` when two choices fit equally well; offer at
most three.

| Video | Good first choices | Why |
|---|---|---|
| Vertical (9:16) talking head, energetic | `word-pop`, `pulse-cut`, `rapid-reveal` | Large words, the spoken word moves, readable on a phone |
| Vertical, calm or premium | `butter-serif`, `minimal-modern`, `soft-script` | Quiet type, less motion |
| Vertical, bold statement clips | `beast-mode`, `bold`, `stacked-emphasis` | Heavy weight, short lines |
| Wide (16:9) interview or tutorial | `classic`, `subtitle`, `minimal-modern` | Low in the frame, small, does not compete with the speaker |
| Text-led pieces with a clear subject | `landmark`, `poster-stack`, `neo-brutal` | Large stacked words, can sit behind the subject |
| One key word per line matters | `spotlight`, `Cascade` | The spoken word is the only bright one |

Rules of thumb:
- One style for the whole video. Mixing styles reads as a mistake.
- Large centred styles hide faces in wide shots. Prefer a bottom style for 16:9 unless the person asks for impact.
- Styles that sit behind the subject need a clear subject. On busy frames, prefer a front style.

## 3. Apply it as one change

Set `captionPresetId` on every caption item in one `apply_timeline` call (one undo step). Remove per-item overrides
that fight the style (`captionFontSize`, `captionTextColor`, `captionHighlightColor`, `captionPosition`) unless the
person set them on purpose. Keep every item `id`.

## 4. Check on frames

1. Run `look` at three moments where captions show: early, middle and late. Use `grid: true`.
2. Check that the words are inside the title-safe box, that no line covers a face, and that the longest line is not
   cut off.
3. If a line runs too long, split that caption item in two at a natural pause rather than shrinking the type.
4. Run `verify_timeline` and fix what it reports.

## 5. Report

Tell the person, in one or two sentences, which style you used and why, and name any caption you split or moved.
