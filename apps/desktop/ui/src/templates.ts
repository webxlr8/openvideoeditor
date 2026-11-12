// Project templates (change 13): a starting point for a kind of video. Each sets the frame shape and hands the
// assistant a brief; the person fills in typed fields (subject, audience, key message, length, frame, media and
// whatever the kind of video needs), the required ones checked before the assistant starts. All text here is our own.

/** One thing the template asks for. */
export interface TemplateField {
  id: string;
  label: string;
  kind: "text" | "textarea" | "select";
  required?: boolean;
  placeholder?: string;
  options?: string[];
  /** Starting value. */
  value?: string;
}

/** What the person typed, by field id. */
export type TemplateValues = Record<string, string>;

const subject = (label: string, placeholder: string): TemplateField => ({
  id: "subject",
  label,
  kind: "text",
  required: true,
  placeholder,
});
const text = (
  id: string,
  label: string,
  placeholder: string,
  required = false,
): TemplateField => ({ id, label, kind: "text", required, placeholder });
const area = (
  id: string,
  label: string,
  placeholder: string,
  required = false,
): TemplateField => ({ id, label, kind: "textarea", required, placeholder });
const audience = (placeholder = "Who it's for…"): TemplateField =>
  text("audience", "Audience", placeholder);
const message = (required = false, placeholder = "The one thing viewers should remember…") =>
  text("message", "Key message", placeholder, required);

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
  /** What this kind of video needs to know (length, frame, media and notes are added for every template). */
  fields: TemplateField[];
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
    fields: [
      subject("Event name", "e.g. Harbour Tech Summit 2026"),
      text("date", "Date", "e.g. 14 March 2026", true),
      text("place", "Place", "e.g. Lisbon", true),
      audience("e.g. attendees and sponsors"),
      message(),
    ],
    brief:
      "Make an event recap. You can't see the footage, so first ask me which shot opens, which closes and which to leave out. Open with a title card naming the event, the date and the place over the opening shot, cut the rest to a steady rhythm (2–4 s per shot), and end on the closing shot with a short closing line. Lower the music under clips with speech. Use the media already in the project; tell me if something is missing.",
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
    fields: [
      subject("Speaker's name", "e.g. Ana Ribeiro"),
      text("role", "Their role", "e.g. founder and CEO of Tidewater", true),
      message(true, "The story in one sentence…"),
      audience("e.g. investors, new hires"),
    ],
    brief:
      "Cut an interview into a founder story. Read the interview from its captions (if there are none yet, ask me to make them with Captions → Generate captions first). Pick the lines that tell problem → turning point → what we do now → why it matters, and cut out false starts and repeated lines. Add a lower third with the speaker's name and role on their first appearance, cover jump cuts with b-roll, and finish on one strong line. When the cut is final, remove the old captions and remind me to generate fresh ones so they match.",
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
    fields: [
      subject("Product name", "e.g. Tidewater 2"),
      area("benefits", "Three benefits", "One per line…", true),
      text("link", "Launch date or link", "e.g. 1 May, tidewater.app", true),
      audience(),
      message(),
    ],
    brief:
      "Make a product launch teaser. Hook in the first two seconds, show the product early, then three short benefit beats with bold on-screen text, and end with the product name, the date or link and the logo. Pace it fast with clean cuts (1–2 s per shot) under the music.",
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
    fields: [
      subject("Show and episode", "e.g. Harbour Talks, episode 12"),
      text("guest", "Guest", "Name and what they do"),
      text("moment", "Roughly where the moment is", "e.g. around 23:10, when we talk about…"),
      audience(),
    ],
    brief:
      "Make a vertical podcast clip. Read the episode from its captions (if there are none yet, ask me to make them with Captions → Generate captions first) and suggest one self-contained moment of 30–60 seconds with a clear hook in its first sentence; check it with me. Trim the episode to that moment and keep only its captions, in a bold word-by-word style. The footage fills the 9:16 frame with a centred crop: tell me to move it in the inspector if the speaker sits off-centre. Add a small title with the show and guest at the top.",
  },
  {
    id: "talking-head-tips",
    name: "Tips in a minute",
    category: "Education",
    aspect: "9:16",
    length: "45–60 s",
    description: "A to-camera video with numbered tips as titles and captions.",
    bring: "A recording of you talking to camera.",
    fields: [
      subject("Topic", "e.g. Better phone photos"),
      area("tips", "Your tips, in order", "One per line…", true),
      audience(),
    ],
    brief:
      "Edit a to-camera tips video. Read it from its captions (if there are none yet, ask me to make them with Captions → Generate captions first), cut out retakes and repeated lines (ask me which take to keep when unsure), and show each tip's number and short title as text when it starts. Punch in slightly (a little more scale) on every other cut to keep it moving. When the cut is final, remove the old captions and remind me to generate fresh ones so they match.",
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
    fields: [
      subject("What it teaches", "e.g. Setting up invoices in Ledgerly"),
      area("steps", "The steps", "One per line…", true),
      audience("e.g. new customers"),
    ],
    brief:
      "Edit a screen tutorial. You can't see the recording, so use the narration's captions (if there are any) and ask me where each step starts and which stretches are dead time or mistakes. Cut those, put a short chapter title at each step, zoom in (scale and position) where I say the text is small, and end with a one-line summary of what was learned. When the cut is final, remind me to generate captions for the narration in the Captions panel.",
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
    fields: [
      subject("Where you went", "e.g. Porto and the Douro valley"),
      text("bookends", "Opening and closing shots", "Which clips open and close it, if you know"),
      audience("e.g. family"),
    ],
    brief:
      "Make a travel diary. Put the media in the order I give (or by file name, which usually follows shooting order), group it by the places I name with a small place-name title at each change, let photos hold 2–3 seconds with a gentle move, apply one consistent warm colour look, and cut to relaxed music. Ask me which shot opens and which closes.",
  },
  {
    id: "highlight-reel",
    name: "Highlight reel",
    category: "Social",
    aspect: "9:16",
    length: "15–30 s",
    description: "The best shots cut hard on the beat for a vertical feed.",
    bring: "Clips (sports, a show, a day out…) and a track.",
    fields: [
      subject("What it's about", "e.g. Saturday's league final"),
      text("mood", "Mood", "e.g. triumphant, fun, intense"),
      audience(),
    ],
    brief:
      "Make a vertical highlight reel from the moments I describe (you can't see the footage, so ask me which clips and where in them). The clips fill the 9:16 frame with a centred crop; tell me which ones I should move in the inspector. Cut short and fast (about 1 s per shot) to the music, use a couple of quick transitions, and put one short title on the opening shot. Keep it under 30 seconds.",
  },
  {
    id: "announcement",
    name: "Announcement",
    category: "Business",
    aspect: "1:1",
    length: "10–20 s",
    description: "A square text-led announcement with animated titles.",
    bring: "Nothing required; a logo or photo if you like.",
    fields: [
      subject("What you're announcing", "e.g. Our new studio opens"),
      text("date", "Date", "e.g. 1 June", true),
      text("link", "Link", "e.g. studio.example.com"),
      audience(),
      message(),
    ],
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
    fields: [
      subject("Who or what it celebrates", "e.g. Grandma Rosa's 90th"),
      area("words", "Words to include", "A title, a dedication…"),
      audience("e.g. the family at the party"),
    ],
    brief:
      "Make a photo montage. Give each photo 3–4 seconds with a slow push or pan, crossfade between them, open with a title and close with a short dedication. Fit the photos to the length of the music, and tell me to adjust any photo whose move crops out a face.",
  },
  {
    id: "explainer",
    name: "Explainer with voice-over",
    category: "Education",
    aspect: "16:9",
    length: "60–90 s",
    description:
      "A script laid out as titles and visuals; you make the voice and images in the Generate panel.",
    bring:
      "A script or the points to cover. Provider keys for voice and images.",
    fields: [
      subject("Topic", "e.g. How heat pumps work"),
      area("points", "The points, or your script", "Paste your script or list the points…", true),
      audience("e.g. homeowners new to the topic"),
      message(),
    ],
    brief:
      "Make an explainer. Tighten the script to about 150 words per minute and give it back to me to record or to make in the Generate panel (Voice); you can't generate media yourself. Plan one visual per sentence: use my media where it fits, and for the rest add a placeholder that describes the image to make in the Generate panel. Add key words as on-screen text and leave room for music under the voice. When the voice-over is in, remind me to generate captions in the Captions panel.",
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
    fields: [
      subject("Product", "e.g. Tidewater 2"),
      text("offer", "The offer", "e.g. 20% off in May", true),
      text("cta", "Call to action", "e.g. Download today at tidewater.app", true),
      audience(),
      message(),
    ],
    brief:
      "Make a 15–30 second ad in 16:9 with a hook, the offer, and a call to action with the logo. Then tell me exactly what to change for 1:1 and 9:16 versions (reframing and title positions) so I can duplicate the project for each.",
  },
];

export const ASPECTS = ["16:9", "9:16", "1:1", "4:5"] as const;

/** Every field the template's dialog shows: its own, then length, frame, media and notes. */
export function templateFields(t: ProjectTemplate): TemplateField[] {
  const lengths = [t.length, "15 s", "30 s", "60 s", "2 min", "3–5 min"];
  return [
    ...t.fields,
    {
      id: "length",
      label: "Length",
      kind: "select",
      options: lengths.filter((l, i) => lengths.indexOf(l) === i),
      value: t.length,
    },
    {
      id: "aspect",
      label: "Frame",
      kind: "select",
      options: [...ASPECTS],
      value: t.aspect,
    },
    {
      id: "media",
      label: "Media you'll add",
      kind: "textarea",
      placeholder: t.bring,
    },
    {
      id: "details",
      label: "Anything else",
      kind: "textarea",
      placeholder: "Anything else the assistant should know…",
    },
  ];
}

/** The starting values: defaults of the select boxes. */
export function templateDefaults(t: ProjectTemplate): TemplateValues {
  const v: TemplateValues = {};
  for (const f of templateFields(t)) if (f.value) v[f.id] = f.value;
  return v;
}

/** Labels of the required fields that are still empty. */
export function missingFields(t: ProjectTemplate, values: TemplateValues): string[] {
  return templateFields(t)
    .filter((f) => f.required && !(values[f.id] ?? "").trim())
    .map((f) => f.label);
}

/** The frame the project starts with. */
export function templateAspect(t: ProjectTemplate, values: TemplateValues): string {
  const a = values.aspect;
  return a && (ASPECTS as readonly string[]).includes(a) ? a : t.aspect;
}

/** The project name for a template and the person's subject. */
export function templateProjectName(t: ProjectTemplate, values: TemplateValues): string {
  const first = (values.subject ?? "")
    .split(/[\n,.]/)[0]
    .replace(/[^\p{L}\p{N} '-]/gu, "")
    .trim()
    .split(/\s+/)
    .slice(0, 5)
    .join(" ");
  return first ? `${t.name} – ${first}`.slice(0, 60) : t.name;
}

/** The first message the assistant receives: the brief, then every filled-in field as a labelled line. */
export function templateMessage(t: ProjectTemplate, values: TemplateValues): string {
  const lines = templateFields(t).flatMap((f) => {
    const v = (values[f.id] ?? "").trim();
    return v ? [`- ${f.label}: ${v.replace(/\n+/g, "\n  ")}`] : [];
  });
  return lines.length ? `${t.brief}\n\nDetails:\n${lines.join("\n")}` : t.brief;
}
