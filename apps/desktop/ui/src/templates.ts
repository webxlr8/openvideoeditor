// Project templates (change 13): a starting point for a kind of video. Each sets the frame shape and hands the
// assistant a brief; the person adds their own details (topic, names, footage). All text here is our own.

export interface ProjectTemplate {
  id: string;
  name: string;
  category: "Social" | "Business" | "Personal" | "Education";
  aspect: "16:9" | "9:16" | "1:1" | "4:5";
  /** Rough length, for the card. */
  length: string;
  description: string;
  /** What the person brings. */
  bring: string;
  /** Placeholder for the details box. */
  ask: string;
  /** The brief the assistant receives, before the person's details. */
  brief: string;
  /** Uses AI generation (images, video, voice or music) — needs a provider key and costs money. */
  generation?: boolean;
}

export const PROJECT_TEMPLATES: ProjectTemplate[] = [
  {
    id: "event-recap",
    name: "Event recap",
    category: "Business",
    aspect: "16:9",
    length: "60–90 s",
    description:
      "The best moments of an event, cut to music with a title and the date.",
    bring: "Footage from the day (and music if you have it).",
    ask: "Event name, date, place, and anything that must be in it…",
    brief:
      "Make an event recap. Open on the strongest wide shot with a title card naming the event, the date and the place. Cut the footage to a steady rhythm (2–4 s per shot), favour faces, reactions and motion, skip shaky or out-of-focus moments, and end on a calm wide shot with a closing line. Keep music under any speech. Use the media already in the project; tell me if something is missing.",
  },
  {
    id: "founder-story",
    name: "Founder story",
    category: "Business",
    aspect: "16:9",
    length: "1–2 min",
    description:
      "An interview cut into a short story: problem, turning point, what's next.",
    bring: "An interview recording and some b-roll.",
    ask: "Who is speaking, their role, and the story in a sentence…",
    brief:
      "Cut an interview into a founder story. Transcribe it, pick the lines that tell problem → turning point → what we do now → why it matters, and drop filler and false starts. Add a lower third with the speaker's name and role on their first appearance, cover jump cuts with b-roll, add captions, and finish on one strong line.",
  },
  {
    id: "product-launch",
    name: "Product launch",
    category: "Business",
    aspect: "16:9",
    length: "30–45 s",
    description:
      "A fast teaser: the problem, the product, three benefits, the call to action.",
    bring: "Product shots or screen recordings, your logo.",
    ask: "Product name, the three benefits, launch date or link…",
    brief:
      "Make a product launch teaser. Hook in the first two seconds, show the product early, then three short benefit beats with bold on-screen text, and end with the product name, the date or link and the logo. Pace it fast with clean cuts on the beat of the music.",
  },
  {
    id: "podcast-clip",
    name: "Podcast clip",
    category: "Social",
    aspect: "9:16",
    length: "30–60 s",
    description:
      "One strong moment from an episode, vertical, with word-by-word captions.",
    bring: "The episode (video or audio).",
    ask: "Show name, guest, and roughly where the moment is…",
    brief:
      "Make a vertical podcast clip. Transcribe the episode and find one self-contained moment of 30–60 seconds with a clear hook in its first sentence. Frame the speaker for 9:16, add bold word-by-word captions, a small title with the show and guest at the top, and remove silences longer than half a second.",
  },
  {
    id: "talking-head-tips",
    name: "Tips in a minute",
    category: "Education",
    aspect: "9:16",
    length: "45–60 s",
    description: "A to-camera video with numbered tips as titles and captions.",
    bring: "A recording of you talking to camera.",
    ask: "The topic and your tips, in order…",
    brief:
      "Edit a to-camera tips video. Remove pauses, retakes and filler, keep a brisk pace, add captions, and show each tip's number and short title as text when it starts. Punch in slightly on every other cut to keep it moving.",
  },
  {
    id: "tutorial",
    name: "Screen tutorial",
    category: "Education",
    aspect: "16:9",
    length: "2–5 min",
    description:
      "A screen recording tightened into clear chapters with titles and zooms.",
    bring: "A screen recording, with or without voice.",
    ask: "What the tutorial teaches and its steps…",
    brief:
      "Edit a screen tutorial. Cut dead time and mistakes, split it into steps with a short chapter title at each, zoom in on the area being clicked when text is small, add captions for the narration, and end with a one-line summary of what was learned.",
  },
  {
    id: "travel-diary",
    name: "Travel diary",
    category: "Personal",
    aspect: "16:9",
    length: "1–3 min",
    description:
      "Your trip in order: place titles, music, and a calm colour look.",
    bring: "Clips and photos from the trip.",
    ask: "Where you went, and anything that should open or close the video…",
    brief:
      "Make a travel diary. Order the media by when it was shot, group it by place with a small place-name title at each change, let photos hold 2–3 seconds with a gentle move, apply one consistent warm colour look, and cut to relaxed music. Open with the best landscape and end with a quiet moment.",
  },
  {
    id: "highlight-reel",
    name: "Highlight reel",
    category: "Social",
    aspect: "9:16",
    length: "15–30 s",
    description: "The best shots cut hard on the beat for a vertical feed.",
    bring: "Clips (sports, a show, a day out…) and a track.",
    ask: "What it's about and the mood…",
    brief:
      "Make a vertical highlight reel. Pick the most energetic moments, reframe them for 9:16, cut on the beat of the music, use a couple of fast transitions at drops, and put one short title on the opening shot. Keep it under 30 seconds.",
  },
  {
    id: "announcement",
    name: "Announcement",
    category: "Business",
    aspect: "1:1",
    length: "10–20 s",
    description: "A square text-led announcement with animated titles.",
    bring: "Nothing required; a logo or photo if you like.",
    ask: "What you're announcing, the date, and the link…",
    brief:
      "Make a square announcement. Use bold animated titles over a simple moving background (a motion template or your media), one idea per card, three or four cards in total, and finish with the date and link. Keep each card on screen long enough to read twice.",
  },
  {
    id: "photo-montage",
    name: "Photo montage",
    category: "Personal",
    aspect: "16:9",
    length: "1–2 min",
    description:
      "Photos with gentle motion, crossfades and music — birthdays, anniversaries, memorials.",
    bring: "Photos (and a song).",
    ask: "Who or what it celebrates, and any words to include…",
    brief:
      "Make a photo montage. Give each photo 3–4 seconds with a slow push or pan, crossfade between them, keep faces in frame, open with a title and close with a short dedication. Time the photo changes to the music phrases.",
  },
  {
    id: "explainer",
    name: "Explainer with voice-over",
    category: "Education",
    aspect: "16:9",
    length: "60–90 s",
    description:
      "A script turned into narration, visuals and titles. Uses AI voice and images.",
    bring:
      "A script or the points to cover. Provider keys for voice and images.",
    ask: "The topic and the points, or paste your script…",
    brief:
      "Make an explainer. Tighten the script to about 150 words per minute, generate the voice-over, and plan one visual per sentence: use my media where it fits and generated images for the rest (tell me the cost before generating). Add key words as on-screen text and captions, and music under the voice.",
    generation: true,
  },
  {
    id: "ad-variations",
    name: "Ad in three shapes",
    category: "Business",
    aspect: "16:9",
    length: "15–30 s",
    description:
      "One ad, then square and vertical versions with titles re-placed for each frame.",
    bring: "Footage or product shots, logo, the offer.",
    ask: "Product, offer, and call to action…",
    brief:
      "Make a 15–30 second ad in 16:9 with a hook, the offer, and a call to action with the logo. Then tell me exactly what to change for 1:1 and 9:16 versions (reframing and title positions) so I can duplicate the project for each.",
  },
];

/** The project name for a template and the person's details. */
export function templateProjectName(
  t: ProjectTemplate,
  details: string,
): string {
  const first = details
    .split(/[\n,.]/)[0]
    .replace(/[^\p{L}\p{N} '-]/gu, "")
    .trim()
    .split(/\s+/)
    .slice(0, 5)
    .join(" ");
  return first ? `${t.name} – ${first}`.slice(0, 60) : t.name;
}

/** The message the assistant receives. */
export function templateMessage(t: ProjectTemplate, details: string): string {
  const d = details.trim();
  return `${t.brief}${d ? `\n\nDetails: ${d}` : ""}`;
}
