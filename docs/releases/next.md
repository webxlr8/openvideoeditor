# Next release (draft)

Draft notes for the release after `v0.1.0-alpha.1`. The version number and date are set when the release is cut.
Everything in [0.1.0-alpha.1](0.1.0-alpha.1.md) still applies unless this page says otherwise.

## New

- **Chat with an assistant you already have.** Claude, Codex, OpenCode, Hermes, Kimi or Grok installed on this
  computer can answer in the app's chat, on your own plan with it: choose **Chat here with …** in the chat. Each turn
  shows what it changed, with **Show** and **Undo this turn**, like the built-in assistant, and shows "uses your …
  plan" where a price would be. If the assistant needs you to sign in, the chat says so; sign in from a terminal and
  send your message again.
- **What such an assistant may do.** It sees and edits the project that is open, and nothing else. It is not given
  files, a terminal or the network through the app, cannot open other projects, import or export, and runs in an
  empty folder. Anything else it asks for is refused and shown in the chat as "Not allowed: …". Stop ends it.
- **The assistants see the project.** The built-in assistant and an installed one can look at frames of the timeline
  and of a clip (contact sheets too) and listen for levels, silences and loudness, before and after they edit.
- **Connect for Codex.** Settings → Assistant gives Codex the app's tools with one button, as for Claude. For Hermes
  the exact command to run is shown.
- **Assistants can read what is said.** A transcript tool gives the built-in assistant and installed ones the timed
  sentences of a clip (transcribed on this computer the first time), so they can cut by what is said.
- **Open Claude or Codex in a terminal on the project.** Settings → Agents & command line opens the assistant in a
  terminal with the app's tools for that session only.
- **Remove silences.** Right-click a clip with sound and choose **Remove silences**: the pauses are cut out, the
  timeline closes up, and one Undo brings them back.
- **Clean up voice.** Right-click a clip with sound and choose **Clean up voice** to take background noise (fans, hum,
  hiss, traffic, room tone) out of speech. It runs on this computer with RNNoise; nothing is uploaded. A video clip is
  muted and a clean copy of its sound plays alongside it; an audio clip switches to the clean copy. One Undo puts it
  back.
- **Waveform and vectorscope.** The Colour tab's scopes switch between the histogram, a luma waveform and a
  vectorscope with colour-bar targets and a skin tone line.
- **Caption styles use the font library.** Each style draws with its own family (serif, display, condensed, script),
  and a caption can pick any font in the inspector.
- **Motion templates fit vertical video.** On a 9:16 project, titles and counters fit the width, the end card stacks
  its frames, and the lower third stays above the area phone apps cover.
- **Choose the library folder.** Settings → Library moves where projects are kept (or goes back to the default).
- **`ove` on the command line.** Settings → Agents & command line adds `ove` to your PATH (Windows) or shows the one
  command to run (macOS).
- **Hardware decoding of AV1** on Windows (D3D11VA), and faster seeking and lower memory use for hardware decoding.
- **Lower memory use** on Home and when no video is shown: the GPU is used only once there is a picture.
- **Faster timeline text** on Windows (printing and applying the timeline as text).

## Privacy (changed)

- The assistant you choose sends the timeline as text, your chat and its tool results to its provider: Anthropic for
  the built-in assistant, or the provider of the assistant installed on your computer.
- Its tool results include frames of your video and sound measurements when it looks or listens. Before, the
  assistant never sent frames.
- Media files themselves never leave your computer. Nothing is sent when no assistant is chosen.
- Unchanged: no account, no telemetry; keys stay in the system keychain; crash reports stay on your computer.

## Fixed

- Timeline: with Fit zoom, clips no longer shift by a few pixels when the track list's scrollbar appears, which could
  make a trim grab land beside the handle.
- Windows installers can be built on Windows itself (they were built on Linux only).
- Diagnostics no longer report a GPU when the viewer could not open it.

## Known limitations (new)

- Kimi and Grok were not available to us to try; their in-app chat follows the Agent Client Protocol but is
  unchecked. Hermes needs a working install of its own runtime.
- An installed assistant's first start can take up to half a minute (Claude and Codex download their adapter once).
