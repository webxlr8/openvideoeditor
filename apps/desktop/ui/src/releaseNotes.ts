// What's new in this release, shown by Home → Changelog. Mirrors the highlights and known limitations in
// docs/releases/0.1.0-alpha.1.md: change both together.

export interface ReleaseHighlight {
  title: string;
  text: string;
}

export interface ReleaseNotes {
  /** The name testers see. */
  name: string;
  tag: string;
  intro: string;
  highlights: ReleaseHighlight[];
  privacy: string[];
  limitations: ReleaseHighlight[];
}

export const RELEASE_NOTES: ReleaseNotes = {
  name: "OpenVideoEditor 0.1.0 alpha 1",
  tag: "v0.1.0-alpha.1",
  intro:
    "The first alpha for invited testers. Expect rough edges. Your project is saved as a new revision after every edit, but keep copies of anything you can't lose.",
  highlights: [
    {
      title: "Edit",
      text: "Import footage, then cut, trim, split, move and stack clips on a multi-track timeline, with undo, redo and a full History you can restore from.",
    },
    {
      title: "Style",
      text: "Titles with 16 animations, 41 effects, 24 transitions, 15 layouts, and colour wheels, curves and LUTs.",
    },
    {
      title: "Captions",
      text: "Made on your computer in 21 styles, with a transcript editor. Speech models download the first time you use them.",
    },
    {
      title: "Motion graphics",
      text: "Sandboxed code items, with 8 templates to start from.",
    },
    {
      title: "Assistant",
      text: "Uses your own Anthropic API key and edits the same timeline you see. Each turn shows what it changed, with Show and Undo this turn. Costs are shown per turn, chat and project, and you can set spend caps in Settings → Assistant.",
    },
    {
      title: "Generate",
      text: "Images, video, voice, music and sound effects with your own fal.ai, OpenAI or ElevenLabs keys. You see the cost first, and nothing is spent until you confirm.",
    },
    {
      title: "Export",
      text: "MP4 (H.264 + AAC) at 720p, 1080p or 4K. Your previous file is only replaced once the new export has finished.",
    },
    {
      title: "Exchange timelines",
      text: "Final Cut Pro, DaVinci Resolve and Premiere Pro (FCPXML, XML, EDL, OpenTimelineIO). Before it writes, the dialog lists anything that won't carry over.",
    },
    {
      title: "Agents and scripts",
      text: "The ove command line and an MCP server for Claude Code, Codex and other agents (ove skills install).",
    },
  ],
  privacy: [
    "No account, no sign-in, no telemetry. Media never leaves your computer.",
    "The assistant sends the timeline as text, your chat and tool results to Anthropic, never media files or frames. Generation sends your prompt and any reference image to the provider you pick.",
    "Keys are kept in the system keychain. Crash reports stay on your computer unless you copy one into a bug report.",
  ],
  limitations: [
    {
      title: "Speed",
      text: "The timeline preview is rendered on the CPU at 540p and 30 fps, so heavy effects or several 4K streams may stutter. There are no proxies yet.",
    },
    {
      title: "Hardware",
      text: "The timeline preview does not use hardware decoding yet (the single-clip viewer does).",
    },
    {
      title: "HDR",
      text: "HLG and PQ footage (such as from an iPhone) is shown and exported tone-mapped to SDR. HDR export is not available.",
    },
    {
      title: "Export formats",
      text: "MP4 with H.264 only, at whole-number frame rates (24–60), so no 23.976 or 29.97. Without a hardware H.264 encoder, export falls back to a lower-quality codec and tells you.",
    },
    {
      title: "Not in this alpha",
      text: "Subject masks (subject and background regions apply to the whole frame), auto-reframe that follows a person, silence and filler-word removal, loudness normalisation, noise reduction, speaker labels in captions, stock media, voice cloning and in-place updates.",
    },
    {
      title: "Interchange",
      text: "Captions, motion graphics, effects, keyframes and transitions don't carry over to other editors. The export dialog lists them.",
    },
  ],
};
