// The UI's only door to the native core. Inside the app this is the generated bindings; in a
// plain browser (UI development, Playwright, unit tests) a mock with the same types stands in.

import { convertFileSrc } from "@tauri-apps/api/core";
import * as generated from "./bindings";
// The effects/transitions/layouts catalogue, written from the core by `export-bindings`.
import fxCatalogJson from "./fx-catalog.json?raw";
import { fire } from "./notify";
import type {
  AgentStatus_Serialize,
  AgentUiEvent_Serialize,
  AppError,
  AutoCaptionRequest_Deserialize,
  ChatItem_Serialize,
  AppInfo_Serialize,
  Diagnostics_Serialize,
  MediaInfo_Serialize,
  PlaybackState,
  TransportCommand,
  ItemView_Serialize,
  ProjectSummary_Serialize,
  ProjectView_Serialize,
  ViewerBackend,
  ViewerRect,
} from "./bindings";
import {
  MOCK_FONTS,
  MOCK_TEMPLATES,
  mockCheck,
  mockFontPreview,
  mockThumbnail,
} from "./motionMock";

export type Commands = typeof generated.commands;
export type Events = typeof generated.events;
type Ok<T> = { status: "ok"; data: T };
type Err = { status: "error"; error: AppError };
export type Result<T> = Ok<T> | Err;

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/** URL of the latest streamed viewer frame (candidate 3 presenter). */
export function frameUrl(): string {
  return convertFileSrc("latest", "ove-frame");
}

// ---------------------------------------------------------------------------------------------
// Browser mock

export interface MockControl {
  /** Crash reports the native side would list. */
  crashReports?: { path: string; at: number; summary: string; seen: boolean }[];
  /** How long a simulated generation job takes (ms). */
  generationMs?: number;
  rects: ViewerRect[];
  lastRect: ViewerRect | null;
  calls: string[];
  /** Simulates the native side routing paths to the window. */
  routePaths(paths: string[]): void;
  /** The simulated assistant: whether a key is stored and how long each streamed step takes. */
  agent: { hasKey: boolean; stepMs: number; ask: boolean };
  /** Simulated speech models: which are on disk, and how long each progress step takes. */
  captions: { installed: string[]; stepMs: number };
}

declare global {
  interface Window {
    __oveMock?: MockControl;
  }
}

const CAPTION_MODELS: [string, string, number][] = [
  ["tiny", "Fast", 77],
  ["base", "Balanced", 147],
  ["small", "Accurate", 487],
];

const MOCK_TRANSCRIPT = [
  "Welcome back to the channel",
  "Today we cut a whole video",
  "with captions made offline",
];

// The style list as the core reports it (a subset of fields is enough for the UI).
const MOCK_CAPTION_PRESETS = (
  [
    [
      "landmark",
      "Landmark",
      "display",
      118,
      400,
      "#ffffff",
      "color",
      "#f2c14e",
      "center",
      "stacked",
      true,
    ],
    [
      "margin-type",
      "Margin Type",
      "sans",
      108,
      700,
      "#f4f1ea",
      "color",
      "#ff6b5a",
      "center",
      "stacked",
      false,
    ],
    [
      "poster-stack",
      "Poster Stack",
      "condensed",
      104,
      800,
      "#ffffff",
      "color",
      "#ffe14d",
      "center",
      "stacked",
      true,
    ],
    [
      "soft-script",
      "Soft Script",
      "script",
      112,
      700,
      "#fff7f0",
      "color",
      "#ffb3c7",
      "center",
      "stacked",
      false,
    ],
    [
      "chrome-italic",
      "Chrome Italic",
      "display",
      116,
      900,
      "#e6ebf2",
      "color",
      "#9ad7ff",
      "center",
      "stacked",
      true,
    ],
    [
      "scrapbook-cutout",
      "Scrapbook Cutout",
      "display",
      106,
      900,
      "#111111",
      "box",
      "#ff5a4e",
      "center",
      "stacked",
      true,
    ],
    [
      "marker-circle",
      "Marker Circle",
      "sans",
      118,
      700,
      "#ffffff",
      "ring",
      "#ff3b30",
      "center",
      "stacked",
      false,
    ],
    [
      "neo-brutal",
      "Neo Brutal",
      "condensed",
      108,
      400,
      "#111111",
      "color",
      "#ff3d7f",
      "center",
      "stacked",
      true,
    ],
    [
      "hero-world-wall",
      "Hero Wall",
      "display",
      104,
      400,
      "#ffffff",
      "color",
      "#7cf2c8",
      "center",
      "stacked",
      true,
    ],
    [
      "Cascade",
      "Cascade",
      "sans",
      50,
      500,
      "#ffffff",
      "color",
      "#a5b4ff",
      "center",
      "reveal",
      false,
    ],
    [
      "rapid-reveal",
      "Rapid Reveal",
      "condensed",
      96,
      800,
      "#ffffff",
      "color",
      "#ffd400",
      "center",
      "reveal",
      true,
    ],
    [
      "stacked-emphasis",
      "Stacked Emphasis",
      "sans",
      56,
      700,
      "#ffffff",
      "color",
      "#4fd1c5",
      "center",
      "stacked",
      false,
    ],
    [
      "word-pop",
      "Word Pop",
      "sans",
      88,
      600,
      "#ffffff",
      "color",
      "#3cff7a",
      "center",
      "word-pop",
      false,
    ],
    [
      "pulse-cut",
      "Pulse Cut",
      "condensed",
      62,
      850,
      "#ffffff",
      "box",
      "#ff2d55",
      "bottom",
      "pulse",
      true,
    ],
    [
      "beast-mode",
      "Beast Mode",
      "display",
      72,
      400,
      "#fffae6",
      "color",
      "#39ff14",
      "center",
      "word-pop",
      true,
    ],
    [
      "butter-serif",
      "Butter Serif",
      "serif",
      52,
      400,
      "#fff4d6",
      "color",
      "#ffd166",
      "bottom",
      "word",
      false,
    ],
    [
      "minimal-modern",
      "Minimal Modern",
      "sans",
      46,
      300,
      "#ffffff",
      "underline",
      "#ffffff",
      "bottom",
      "word",
      false,
    ],
    [
      "spotlight",
      "Spotlight",
      "sans",
      60,
      700,
      "#ffffff",
      "box",
      "#ffffff",
      "center",
      "spotlight",
      false,
    ],
    [
      "classic",
      "Classic",
      "sans",
      40,
      500,
      "#ffffff",
      "none",
      null,
      "bottom",
      "word",
      false,
    ],
    [
      "subtitle",
      "Subtitle",
      "sans",
      40,
      500,
      "#fff6c4",
      "none",
      null,
      "bottom",
      "word",
      false,
    ],
    [
      "bold",
      "Bold",
      "sans",
      60,
      800,
      "#ffffff",
      "color",
      "#ffd400",
      "bottom",
      "word",
      false,
    ],
  ] as [
    string,
    string,
    string,
    number,
    number,
    string,
    string,
    string | null,
    string,
    string,
    boolean,
  ][]
).map(
  ([
    id,
    label,
    family,
    fontSize,
    fontWeight,
    textColor,
    kind,
    color,
    position,
    animation,
    uppercase,
  ]) => ({
    id,
    label,
    family,
    fontSize,
    fontWeight,
    textColor,
    highlightColor: color,
    mark: {
      kind,
      color: color ?? undefined,
      text:
        kind === "box"
          ? color === "#ffffff"
            ? "#111111"
            : "#ffffff"
          : undefined,
    },
    position,
    align: id === "margin-type" ? "left" : "center",
    uppercase,
    lowercase: id === "margin-type",
    letterSpacing: 0,
    animation,
    revealMs: 160,
    stroke: [
      "bold",
      "word-pop",
      "beast-mode",
      "subtitle",
      "rapid-reveal",
    ].includes(id)
      ? { color: "#111111", width: 0.08 }
      : null,
    shadow: { color: "#000000a0", x: 0, y: 0.05 },
    panel:
      id === "classic"
        ? { color: "#000000a0", perWord: false, radius: 0.18 }
        : id === "neo-brutal"
          ? { color: "#ffe600", perWord: false, radius: 0 }
          : id === "scrapbook-cutout"
            ? { color: "#fff3d6", perWord: true, radius: 0.04 }
            : null,
    composition:
      animation === "stacked" && id !== "stacked-emphasis"
        ? "behind-subject"
        : "front",
  }),
);

type Listener = (event: { payload: unknown }) => void;

function createMock(): {
  commands: Commands;
  events: Events;
  control: MockControl;
} {
  const listeners = new Map<string, Set<Listener>>();
  const emit = (name: string, payload: unknown) =>
    listeners.get(name)?.forEach((cb) => cb({ payload }));
  const ok = <T>(data: T): Promise<Ok<T>> =>
    Promise.resolve({ status: "ok", data });
  const fail = (code: AppError["code"], message: string): Promise<Err> =>
    Promise.resolve({ status: "error", error: { code, message } });

  const state: PlaybackState = {
    hasMedia: false,
    frame: 0,
    totalFrames: 0,
    frameRate: 30,
    playing: false,
    speed: 0,
    looping: false,
    volume: 1,
    muted: false,
    audioAvailable: false,
  };
  let timer: ReturnType<typeof setInterval> | null = null;
  const last = () => Math.max(0, state.totalFrames - 1);
  const changed = () => emit("playback-changed", { ...state });
  const setSpeed = (s: number) => {
    state.speed = s;
    state.playing = s !== 0;
    if (timer) clearInterval(timer);
    timer = null;
    if (state.playing) {
      timer = setInterval(
        () => {
          const dir = Math.sign(state.speed ?? 0);
          const next = state.frame + dir;
          if (next < 0 || next > last()) {
            if (state.looping) state.frame = dir > 0 ? 0 : last();
            else setSpeed(0);
          } else state.frame = next;
          changed();
        },
        1000 / ((state.frameRate ?? 30) * Math.abs(state.speed ?? 1)),
      );
    }
  };
  const apply = (c: TransportCommand) => {
    if (!state.hasMedia) {
      if (c.kind === "setLoop") state.looping = c.enabled;
      return;
    }
    switch (c.kind) {
      case "play":
        if (state.frame >= last() && !state.looping) state.frame = 0;
        setSpeed(1);
        break;
      case "pause":
        setSpeed(0);
        break;
      case "togglePlay":
        apply({ kind: state.playing ? "pause" : "play" });
        break;
      case "step":
        setSpeed(0);
        state.frame = Math.min(Math.max(0, state.frame + c.frames), last());
        break;
      case "goToStart":
        setSpeed(0);
        state.frame = 0;
        break;
      case "goToEnd":
        setSpeed(0);
        state.frame = last();
        break;
      case "shuttleForward":
        setSpeed((state.speed ?? 0) >= 1 ? 2 : 1);
        break;
      case "shuttleReverse":
        setSpeed((state.speed ?? 0) <= -1 ? -2 : -1);
        break;
      case "seekFrame":
        state.frame = Math.min(c.frame, last());
        break;
      case "setLoop":
        state.looping = c.enabled;
        break;
    }
  };

  // Generation mock state.
  const genKeys = new Set<string>();
  const genJobs: {
    id: string;
    kind: string;
    model: string;
    request: unknown;
    status: string;
    progress: number;
    error: null;
    estimateUsd: number;
    createdAt: number;
  }[] = [];
  const genCatalog = () => ({
    models: [
      {
        id: "fal:flux-dev",
        provider: "fal",
        kind: "image",
        label: "FLUX.1 [dev]",
        durations: [],
        maxSeconds: 0,
        aspects: ["16:9", "9:16", "1:1"],
        default: true,
        price: { per: "image", usd: 0.025 },
      },
      {
        id: "fal:kling-2.1",
        provider: "fal",
        kind: "video",
        label: "Kling 2.1",
        durations: [5, 10],
        maxSeconds: 10,
        aspects: ["16:9", "9:16", "1:1"],
        default: true,
        price: { per: "second", usd: 0.056 },
      },
      {
        id: "elevenlabs:multilingual-v2",
        provider: "elevenlabs",
        kind: "voice",
        label: "Eleven Multilingual v2",
        durations: [],
        maxSeconds: 0,
        aspects: [],
        default: true,
        price: { per: "kiloChar", usd: 0.3 },
      },
      {
        id: "elevenlabs:music",
        provider: "elevenlabs",
        kind: "music",
        label: "Eleven Music",
        durations: [],
        maxSeconds: 300,
        aspects: [],
        default: true,
        price: { per: "second", usd: 0.01 },
      },
      {
        id: "elevenlabs:sound-effects",
        provider: "elevenlabs",
        kind: "sfx",
        label: "Sound effects",
        durations: [],
        maxSeconds: 30,
        aspects: [],
        default: true,
        price: { per: "second", usd: 0.004 },
      },
    ],
    keys: [
      ["fal", "fal.ai", "https://fal.ai/dashboard/keys", "FAL_KEY"],
      [
        "openai",
        "OpenAI",
        "https://platform.openai.com/api-keys",
        "OPENAI_API_KEY",
      ],
      [
        "elevenlabs",
        "ElevenLabs",
        "https://elevenlabs.io/app/settings/api-keys",
        "ELEVENLABS_API_KEY",
      ],
    ].map(([provider, label, keyUrl, envVar]) => ({
      provider,
      label,
      keyUrl,
      envVar,
      hasKey: genKeys.has(provider),
      source: genKeys.has(provider) ? "keychain" : null,
    })),
  });
  const control: MockControl = {
    rects: [],
    lastRect: null,
    calls: [],
    routePaths(paths) {
      emit("open-paths-requested", { paths });
    },
    agent: { hasKey: true, stepMs: 120, ask: false },
    captions: { installed: [], stepMs: 60 },
  };
  const track = (name: string) => control.calls.push(name);

  const media = (path: string): MediaInfo_Serialize => ({
    path,
    name: path.split(/[\\/]/).pop() ?? path,
    durationSeconds: 60,
    width: 1920,
    height: 1080,
    frameRate: 30,
    totalFrames: 1800,
    videoCodec: "test pattern",
    audioLayout: null,
    placeholder: true,
  });
  const backend: ViewerBackend = "stream";

  // In-memory project store: one revision per edit, snapshot-based undo/redo.
  const library: ProjectView_Serialize[] = [];
  let project: ProjectView_Serialize | null = null;
  let undoStack: ProjectView_Serialize[] = [];
  let redoStack: ProjectView_Serialize[] = [];
  let seq = 0;
  const clone = (p: ProjectView_Serialize) =>
    JSON.parse(JSON.stringify(p)) as ProjectView_Serialize;
  const withFlags = (p: ProjectView_Serialize): ProjectView_Serialize => ({
    ...p,
    canUndo: undoStack.length > 0,
    canRedo: redoStack.length > 0,
    durationSeconds: Math.max(0, ...p.items.map((i) => i.end ?? 0)),
  });
  // The timeline viewer follows edits, as the engine's timeline session does.
  let timelineOpen = false;
  const timelineFrames = () =>
    Math.max(1, Math.ceil((project?.durationSeconds ?? 0) * 30));
  const publish = () => {
    if (!project)
      return ok<ProjectView_Serialize>(
        null as unknown as ProjectView_Serialize,
      );
    project = withFlags(project);
    if (timelineOpen) {
      state.totalFrames = timelineFrames();
      state.frame = Math.min(state.frame, last());
      changed();
    }
    const i = library.findIndex((x) => x.bundle === project!.bundle);
    if (i >= 0) library[i] = project;
    emit("project-changed", clone(project));
    return ok(clone(project));
  };
  const editProject = (
    summary: string,
    f: (p: ProjectView_Serialize) => void,
  ) => {
    if (!project) return fail("invalid_argument", "No project is open.");
    undoStack.push(clone(project));
    redoStack = [];
    f(project);
    project.rev = (project.rev ?? 0) + 1;
    project.undoSummary = summary;
    return publish();
  };
  const newProject = (name: string): ProjectView_Serialize => ({
    id: `p_${++seq}`,
    name,
    bundle: `/mock/library/${name}.ove`,
    rev: Date.now(),
    aspectRatio: "16:9",
    durationSeconds: 0,
    canUndo: false,
    canRedo: false,
    undoSummary: null,
    tracks: [
      ["t_text", "Text", "video"],
      ["t_captions", "Captions", "caption"],
      ["t_overlay", "Overlay", "video"],
      ["t_main", "Main", "video"],
      ["t_audio", "Audio", "audio"],
    ].map(([id, name, kind]) => ({
      id,
      name,
      kind,
      locked: false,
      visible: true,
      muted: false,
    })),
    items: [],
    media: [],
  });
  // Like the real import: unreadable files are skipped and reported through `appErrorRaised`.
  const importPaths = (all: string[]) => {
    const playable =
      /\.(mp4|m4v|mov|mxf|mkv|webm|m4a|wav|caf|opus|ogg|aac|png|jpe?g|webp)$/i;
    const paths = all.filter((p) => playable.test(p));
    const skipped = all.filter((p) => !playable.test(p));
    if (skipped.length > 0)
      emit("app-error-raised", {
        code: "unsupported",
        message: skipped
          .map(
            (p) =>
              `“${p.split(/[\\/]/).pop()}” can't be opened: this file type is not supported.`,
          )
          .join(" "),
      });
    if (paths.length === 0) return publish();
    return importValid(paths);
  };
  const importValid = (paths: string[]) =>
    editProject(
      `import ${paths.length} file${paths.length === 1 ? "" : "s"}`,
      (p) => {
        for (const path of paths) {
          const n = ++seq;
          const name = path.split(/[\\/]/).pop() ?? path;
          const audio = /\.(m4a|wav|caf|opus|ogg|aac)$/i.test(path);
          const track = audio ? "t_audio" : "t_main";
          const end = Math.max(
            0,
            ...p.items
              .filter((i) => i.trackId === track)
              .map((i) => i.end ?? 0),
          );
          p.media.push({
            id: `m${n}`,
            name,
            path,
            kind: audio ? "audio" : "video",
            durationSeconds: 60,
            width: audio ? null : 1920,
            height: audio ? null : 1080,
            status: "online",
            thumbnail: null,
          });
          p.items.push(
            fromDetail({
              id: `i${n}`,
              trackId: track,
              type: audio ? "audio" : "video",
              name,
              startTime: end,
              endTime: end + 60,
              mediaItemId: `m${n}`,
            }),
          );
        }
      },
    );
  const findItem = (id: string): ItemView_Serialize | undefined =>
    project?.items.find((i) => i.id === id);

  // Full item fields (document names) ride along on each mock item, as the real core keeps them in the document.
  type Fields = Record<string, unknown>;
  type MockItem = ItemView_Serialize & { fields: Fields };
  const HEAD = [
    "id",
    "trackId",
    "type",
    "startTime",
    "endTime",
    "name",
    "trimStart",
    "mediaItemId",
    "pending",
  ];
  const n = (v: unknown, d = 0) => (typeof v === "number" ? v : d);
  const refresh = (i: MockItem) => {
    const f = i.fields;
    const tr = (k: string) => n((f[k] as Fields | undefined)?.duration);
    i.hidden = f.hidden === true;
    i.muted = f.muted === true;
    i.text =
      typeof f.textContent === "string"
        ? f.textContent
        : typeof f.captionText === "string"
          ? f.captionText
          : null;
    i.transitionIn = tr("transitionIn");
    i.transitionOut = tr("transitionOut");
    i.speed = n(f.playbackSpeed, 1);
    return i;
  };
  function fromDetail(d: Fields): MockItem {
    const fields: Fields = {};
    for (const [k, v] of Object.entries(d))
      if (!HEAD.includes(k)) fields[k] = v;
    return refresh({
      id: typeof d.id === "string" ? d.id : `i${++seq}`,
      trackId: String(d.trackId),
      kind: String(d.type),
      name: String(d.name ?? d.textContent ?? d.type),
      start: n(d.startTime),
      end: n(d.endTime),
      trimStart: n(d.trimStart),
      mediaId: typeof d.mediaItemId === "string" ? d.mediaItemId : null,
      pending: d.pending === true,
      hidden: false,
      muted: false,
      text: null,
      transitionIn: 0,
      transitionOut: 0,
      speed: 1,
      fields,
    });
  }
  const toDetail = (i: MockItem): Fields => ({
    id: i.id,
    trackId: i.trackId,
    type: i.kind,
    startTime: i.start,
    endTime: i.end,
    name: i.name,
    trimStart: i.trimStart,
    ...(i.mediaId ? { mediaItemId: i.mediaId } : {}),
    ...i.fields,
  });
  const mock = (id: string) => findItem(id) as MockItem | undefined;
  const applyEdit = (id: string, set: Fields, unset: string[]) => {
    const i = mock(id);
    if (!i) return;
    for (const k of unset) delete i.fields[k];
    for (const [k, v] of Object.entries(set)) {
      if (k === "startTime") i.start = n(v);
      else if (k === "endTime") i.end = n(v);
      else if (k === "trackId") i.trackId = String(v);
      else if (k === "name") i.name = String(v);
      else if (k === "trimStart") i.trimStart = n(v);
      else if (k === "mediaItemId") i.mediaId = String(v);
      else i.fields[k] = v;
    }
    refresh(i);
  };

  // A captions job: model download (when not installed), then transcription, then `finish`.
  let captionJob: ReturnType<typeof setInterval> | null = null;
  const runCaptionJob = (model: string, finish: () => void) => {
    const steps: [string, number, string][] = [];
    const sizeMb = CAPTION_MODELS.find(([id]) => id === model)?.[2] ?? 148;
    if (!control.captions.installed.includes(model))
      for (let k = 1; k <= 5; k++)
        steps.push([
          "download",
          k / 5,
          `Downloading the ${model} speech model (${Math.round((sizeMb * k) / 5)} of ${sizeMb} MB)`,
        ]);
    for (let k = 1; k <= 4; k++)
      steps.push(["transcribe", k / 4, "Transcribing talk.mp4"]);
    let k = 0;
    captionJob = setInterval(() => {
      const [stage, fraction, message] = steps[k++];
      if (stage === "transcribe" && !control.captions.installed.includes(model))
        control.captions.installed.push(model);
      emit("caption-job-progress", {
        stage,
        fraction,
        message,
        done: false,
        error: null,
        cancelled: false,
        captions: null,
        language: null,
        secondsTaken: null,
      });
      if (k >= steps.length) {
        if (captionJob) clearInterval(captionJob);
        captionJob = null;
        if (!control.captions.installed.includes(model))
          control.captions.installed.push(model);
        finish();
      }
    }, control.captions.stepMs);
  };

  let exportTimer: ReturnType<typeof setInterval> | null = null;
  const stopExport = () => {
    if (exportTimer) clearInterval(exportTimer);
    exportTimer = null;
  };

  // Simulated assistant: streams a reply, "calls" tools, and adds a title as one undo step.
  const MODELS = [
    { id: "claude-sonnet-5-5", label: "Claude Sonnet 5.5" },
    { id: "claude-opus-5-5", label: "Claude Opus 5.5" },
    { id: "claude-haiku-5-5", label: "Claude Haiku 5.5" },
  ];
  let agentModel = MODELS[0].id;
  const chats = new Map<
    string,
    { title: string; updatedAt: number; items: ChatItem_Serialize[] }
  >();
  let turn: { stop: boolean; answer?: (a: string) => void } | null = null;
  const agentStatus = (): AgentStatus_Serialize => ({
    hasKey: control.agent.hasKey,
    keySource: control.agent.hasKey ? "keychain" : null,
    model: agentModel,
    models: MODELS,
    running: turn !== null,
  });
  const agentEvent = (payload: AgentUiEvent_Serialize) =>
    emit("agent-ui-event", payload);
  const wait = () => new Promise((r) => setTimeout(r, control.agent.stepMs));
  const item = (
    role: string,
    text: string,
    id: string | null = null,
    ok: boolean | null = null,
  ): ChatItem_Serialize => ({ role, text, id, ok, options: [] });
  const runTurn = async (chatId: string) => {
    const chat = chats.get(chatId)!;
    const t = turn!;
    const started = Date.now();
    const say = async (text: string) => {
      const it = item("assistant", "");
      chat.items.push(it);
      for (const word of text.split(/(?<= )/)) {
        if (t.stop) return;
        await wait();
        it.text += word;
        agentEvent({ kind: "textDelta", text: word });
      }
    };
    const tool = async (
      id: string,
      name: string,
      label: string,
      run: () => string,
    ) => {
      if (t.stop) return;
      const it = item("tool", label, id);
      chat.items.push(it);
      agentEvent({ kind: "toolCall", id, name, label });
      await wait();
      const summary = run();
      it.ok = true;
      agentEvent({ kind: "toolResult", id, ok: true, summary });
    };
    agentEvent({ kind: "turnStarted", chatId });
    agentEvent({ kind: "thinkingDelta", text: "…" });
    await wait();
    await say("Let me look at the timeline first. ");
    await tool(
      "tu_print",
      "print_timeline",
      "Read the timeline",
      () => `${project?.items.length ?? 0} items`,
    );
    if (control.agent.ask && !t.stop) {
      const q = item("question", "Which style should the title use?", "tu_ask");
      q.options = ["Bold", "Minimal"];
      chat.items.push(q);
      agentEvent({
        kind: "question",
        id: "tu_ask",
        question: q.text,
        options: q.options,
      });
      const a = await new Promise<string>((r) => (t.answer = r));
      q.text += `\n→ ${a}`;
      q.ok = true;
    }
    await tool("tu_apply", "apply_timeline", "Edit the timeline", () => {
      fire(
        editProject("assistant: add opening title", (p) => {
          p.items.push(
            fromDetail({
              id: `i${++seq}`,
              trackId: "t_text",
              type: "text",
              name: "Opening title",
              startTime: 0,
              endTime: 3,
              textContent: "Welcome",
            }),
          );
        }),
      );
      return "added 1";
    });
    await tool(
      "tu_verify",
      "verify_timeline",
      "Check the timeline",
      () => "no problems",
    );
    await tool("tu_seek", "seek_viewer", "Show the title", () => {
      agentEvent({ kind: "seek", seconds: 1.5 });
      return "Showing 1.50s.";
    });
    await say(
      "Done: I added a 3-second **Welcome** title at the start. Undo removes it in one step.",
    );
    chat.updatedAt = Date.now();
    turn = null;
    agentEvent({
      kind: "turnDone",
      chatId,
      stopped: t.stop,
      durationMs: Date.now() - started,
      inputTokens: 5400,
      outputTokens: 320,
      cacheReadTokens: 4800,
      costUsd: 0.0148,
      note: null,
    });
  };

  const commands: Commands = {
    appInfo: () =>
      ok<AppInfo_Serialize>({
        version: "dev",
        os: "Browser",
        arch: "web",
        gpuAdapter: null,
        decodeCapabilities: "mock",
        viewerBackend: backend,
        logDir: null,
        libraryDir: "/mock/library",
      }),
    takePendingOpenPaths: () => (
      track("takePendingOpenPaths"),
      ok<string[]>([])
    ),
    openMedia: (path: string) => {
      track(`openMedia:${path}`);
      if (!/\.(mp4|m4v|mov|mxf|mkv|webm|m4a|wav|caf|opus|ogg|aac)$/i.test(path))
        return fail(
          "unsupported",
          `“${path.split(/[\\/]/).pop()}” can't be opened: this file type is not supported.`,
        );
      const info = media(path);
      timelineOpen = false;
      setSpeed(0);
      Object.assign(state, {
        hasMedia: true,
        frame: 0,
        totalFrames: info.totalFrames,
        frameRate: 30,
      });
      changed();
      return ok(info);
    },
    openMediaDialog: () => commands.openMedia("/mock/Sample clip.mov"),
    closeMedia: () => {
      timelineOpen = false;
      setSpeed(0);
      Object.assign(state, { hasMedia: false, frame: 0, totalFrames: 0 });
      changed();
      return ok({ ...state });
    },
    setViewerRect: (rect: ViewerRect) => {
      control.rects.push(rect);
      control.lastRect = rect;
      return ok(null);
    },
    transport: (command: TransportCommand) => {
      track(`transport:${command.kind}`);
      apply(command);
      changed();
      return ok({ ...state });
    },
    setVolume: (volume: number | null, muted: boolean) => {
      state.volume = Math.min(1, Math.max(0, volume ?? 1));
      state.muted = muted;
      changed();
      return ok({ ...state });
    },
    playbackState: () => ok({ ...state }),
    diagnostics: () =>
      ok<Diagnostics_Serialize>({
        decodePath: state.hasMedia ? "test pattern (mock)" : "none",
        viewer: {
          backend,
          gpuAdapter: null,
          presentedFrames: 0,
          frameWidth: 0,
          frameHeight: 0,
          pixelFormat: null,
          copiesLastFrame: 0,
          lastPresentMs: 0,
          deviceRecoveries: 0,
          physicalWidth: 0,
          physicalHeight: 0,
          imageX: 0,
          imageY: 0,
          imageWidth: 0,
          imageHeight: 0,
          lastSeekMs: null,
        },
        droppedFrames: 0,
        avOffsetMs: null,
        processMemoryBytes: null,
        audioAvailable: false,
      }),
    simulateDeviceLoss: () => ok(null),
    debugPanicWorker: () =>
      fail(
        "worker_panic",
        "An internal error stopped a test worker. Details are in the log.",
      ),
    debugPanicViewer: () => ok(null),
    listProjects: () =>
      ok<ProjectSummary_Serialize[]>(
        library.map((p) => ({
          bundle: p.bundle,
          id: p.id,
          name: p.name,
          updatedAt: Date.now(),
          aspectRatio: p.aspectRatio,
          durationSeconds: p.durationSeconds,
          itemCount: p.items.length,
          damaged: null,
          thumbnail: null,
        })),
      ),
    createProject: (name: string) => {
      project = newProject(name || "Untitled Project");
      undoStack = [];
      redoStack = [];
      library.unshift(project);
      return publish();
    },
    openProject: (bundle: string) => {
      const p = library.find((x) => x.bundle === bundle);
      if (!p) return fail("not_found", "That project no longer exists.");
      project = clone(p);
      undoStack = [];
      redoStack = [];
      return publish();
    },
    closeProject: () => {
      project = null;
      timelineOpen = false;
      emit("project-changed", null);
      return ok(null);
    },
    projectView: () => ok(project ? clone(project) : null),
    importMedia: (paths: string[]) => importPaths(paths),
    importMediaDialog: () => importPaths(["/mock/Sample clip.mov"]),
    moveItem: (id: string, start: number | null, trackId: string | null) =>
      editProject("move clip", () => {
        const i = findItem(id);
        if (!i) return;
        const d = (i.end ?? 0) - (i.start ?? 0);
        i.start = Math.max(0, start ?? 0);
        i.end = i.start + d;
        if (trackId) i.trackId = trackId;
      }),
    removeItems: (ids: string[]) =>
      editProject(
        `delete ${ids.length} item${ids.length === 1 ? "" : "s"}`,
        (p) => {
          p.items = p.items.filter((i) => !ids.includes(i.id));
        },
      ),
    splitItem: (id: string, at: number | null) =>
      editProject("split clip", (p) => {
        const i = findItem(id);
        if (!i || at === null || at <= (i.start ?? 0) || at >= (i.end ?? 0))
          return;
        p.items.push({
          ...i,
          id: `${i.id}_b`,
          start: at,
          trimStart: (i.trimStart ?? 0) + at - (i.start ?? 0),
        });
        i.end = at;
      }),
    setAspectRatio: (aspect: string) =>
      editProject("change aspect ratio", (p) => void (p.aspectRatio = aspect)),
    renameProject: (name: string) =>
      editProject("rename project", (p) => void (p.name = name)),
    setTrackFlag: (id: string, flag: string, value: boolean) =>
      editProject("track setting", (p) => {
        const t = p.tracks.find((x) => x.id === id);
        if (t && (flag === "muted" || flag === "visible" || flag === "locked"))
          t[flag] = value;
      }),
    undo: () => {
      if (!project || undoStack.length === 0)
        return fail("invalid_argument", "Nothing to undo.");
      redoStack.push(clone(project));
      project = undoStack.pop()!;
      return publish();
    },
    redo: () => {
      if (!project || redoStack.length === 0)
        return fail("invalid_argument", "Nothing to redo.");
      undoStack.push(clone(project));
      project = redoStack.pop()!;
      return publish();
    },
    itemDetail: (id: string) => {
      const i = mock(id);
      return i
        ? ok(JSON.stringify(toDetail(i)))
        : fail("not_found", "That item no longer exists.");
    },
    updateItem: (
      id: string,
      setJson: string,
      unset: string[],
      summary: string | null,
    ) =>
      editProject(summary ?? "edit item", () =>
        applyEdit(id, JSON.parse(setJson) as Fields, unset),
      ),
    updateItems: (editsJson: string, summary: string | null) =>
      editProject(summary ?? "edit items", () => {
        for (const e of JSON.parse(editsJson) as {
          id: string;
          set?: Fields;
          unset?: string[];
        }[])
          applyEdit(e.id, e.set ?? {}, e.unset ?? []);
      }),
    addItems: (itemsJson: string, summary: string | null) =>
      editProject(summary ?? "add items", (p) => {
        for (const d of JSON.parse(itemsJson) as Fields[])
          p.items.push(fromDetail({ ...d, id: d.id ?? `i${++seq}` }));
      }),
    duplicateItems: (ids: string[]) =>
      editProject("duplicate", (p) => {
        for (const id of ids) {
          const i = mock(id);
          if (!i) continue;
          const len = (i.end ?? 0) - (i.start ?? 0);
          p.items.push(
            fromDetail({
              ...toDetail(i),
              id: `i${++seq}`,
              startTime: i.end,
              endTime: (i.end ?? 0) + len,
            }),
          );
        }
      }),
    pasteItems: (itemsJson: string, at: number | null) =>
      editProject("paste", (p) => {
        const list = JSON.parse(itemsJson) as Fields[];
        const min = Math.min(...list.map((d) => n(d.startTime)));
        for (const d of list)
          p.items.push(
            fromDetail({
              ...d,
              id: `i${++seq}`,
              startTime: (at ?? 0) + n(d.startTime) - min,
              endTime: (at ?? 0) + n(d.endTime) - min,
            }),
          );
      }),
    trimItem: (id: string, edge: string, time: number | null) =>
      editProject("trim", () => {
        const i = mock(id);
        if (!i || time === null) return;
        if (edge === "start") {
          const s = Math.min(Math.max(0, time), (i.end ?? 0) - 0.04);
          const trim = Math.max(0, (i.trimStart ?? 0) + (s - (i.start ?? 0)));
          i.trimStart = trim;
          i.start = s;
        } else i.end = Math.max((i.start ?? 0) + 0.04, time);
      }),
    addTrack: (kind: string, name: string) =>
      editProject("add track", (p) => {
        const t = {
          id: `t${++seq}`,
          name,
          kind,
          locked: false,
          visible: true,
          muted: false,
        };
        if (kind === "audio") p.tracks.push(t);
        else p.tracks.unshift(t);
      }),
    renameTrack: (id: string, name: string) =>
      editProject("rename track", (p) => {
        const t = p.tracks.find((x) => x.id === id);
        if (t) t.name = name;
      }),
    reorderTracks: (order: string[]) =>
      editProject("reorder tracks", (p) => {
        p.tracks = order
          .map((id) => p.tracks.find((t) => t.id === id)!)
          .filter(Boolean);
      }),
    openTimeline: () => {
      track("openTimeline");
      if (!project) return fail("invalid_argument", "No project is open.");
      const [width, height] =
        project.aspectRatio === "9:16"
          ? [1080, 1920]
          : project.aspectRatio === "1:1"
            ? [1080, 1080]
            : [1920, 1080];
      timelineOpen = true;
      setSpeed(0);
      Object.assign(state, {
        hasMedia: true,
        frame: 0,
        totalFrames: timelineFrames(),
        frameRate: 30,
      });
      changed();
      return ok<MediaInfo_Serialize>({
        path: project.bundle,
        name: project.name,
        durationSeconds: state.totalFrames / 30,
        width,
        height,
        frameRate: 30,
        totalFrames: state.totalFrames,
        videoCodec: "timeline",
        audioLayout: "stereo mix",
        placeholder: true,
      });
    },
    exportProject: (request: { path: string; height: number; fps: number }) => {
      track(`exportProject:${request.height}p${request.fps}`);
      if (!project) return fail("invalid_argument", "No project is open.");
      if (exportTimer)
        return fail("invalid_argument", "An export is already running.");
      const total = Math.max(
        1,
        Math.round((project.durationSeconds ?? 0) * request.fps),
      );
      let frame = 0;
      const started = Date.now();
      exportTimer = setInterval(() => {
        frame = Math.min(total, frame + Math.ceil(total / 10));
        const done = frame >= total;
        emit("export-progress", {
          fraction: frame / total,
          frame,
          totalFrames: total,
          done,
          error: null,
          path: done ? request.path : null,
          encoder: done ? "mock" : null,
          secondsTaken: done ? (Date.now() - started) / 1000 : null,
        });
        if (done) stopExport();
      }, 30);
      return ok(null);
    },
    cancelExport: () => {
      track("cancelExport");
      if (exportTimer) {
        stopExport();
        emit("export-progress", {
          fraction: null,
          frame: 0,
          totalFrames: 0,
          done: true,
          error: "Export cancelled.",
          path: null,
          encoder: null,
          secondsTaken: null,
        });
      }
      return ok(null);
    },
    chooseExportPath: (defaultName: string) => (
      track("chooseExportPath"),
      ok<string | null>(`/mock/exports/${defaultName}.mp4`)
    ),
    revealPath: (path: string) => (track(`revealPath:${path}`), ok(null)),
    crashReports: () => Promise.resolve(control.crashReports ?? []),
    markCrashReportsSeen: () => {
      for (const r of control.crashReports ?? []) r.seen = true;
      return ok(null);
    },
    readCrashReport: (path: string) => ok(`report ${path}`),
    checkForUpdates: () =>
      ok({
        current: "0.1.0",
        latest: "0.2.0",
        available: true,
        notes: "Faster export.",
        url: "https://example.invalid/download",
        message: null,
      }),
    openUrl: (url: string) => (track(`openUrl:${url}`), ok(null)),
    // Generation (change 14): a catalog, keys in memory, jobs that finish after a moment.
    generationCatalog: () => Promise.resolve(JSON.stringify(genCatalog())),
    setProviderKey: (provider: string, key: string) => {
      track(`setProviderKey:${provider}`);
      if (key.trim().length < 16)
        return fail("invalid_argument", "That doesn't look like an API key.");
      genKeys.add(provider);
      return ok(JSON.stringify(genCatalog()));
    },
    clearProviderKey: (provider: string) => {
      genKeys.delete(provider);
      return ok(JSON.stringify(genCatalog()));
    },
    planGeneration: (kind: string, request: string) => {
      if (!project) return fail("invalid_argument", "Open a project first.");
      const r = JSON.parse(request) as {
        prompt: string;
        duration?: number | null;
        start?: number | null;
      };
      if (!r.prompt?.trim())
        return fail("invalid_argument", "the prompt is empty");
      const audio = kind === "voice" || kind === "music" || kind === "sfx";
      const seconds =
        kind === "voice"
          ? Math.max(1, r.prompt.length / 15)
          : (r.duration ?? (kind === "video" ? 5 : kind === "music" ? 30 : 5));
      const est =
        kind === "image"
          ? 0.025
          : kind === "video"
            ? 0.056 * seconds
            : kind === "voice"
              ? (0.3 * r.prompt.length) / 1000
              : kind === "music"
                ? 0.01 * seconds
                : 0.004 * seconds;
      return ok(
        JSON.stringify({
          kind,
          aspect: project.aspectRatio,
          seconds,
          estimateUsd: Math.round(est * 1000) / 1000,
          trackId: audio ? "t_audio" : "t_overlay",
          start: r.start ?? 0,
          itemId: "preview",
        }),
      );
    },
    startGeneration: (kind: string, request: string) => {
      track(`startGeneration:${kind}`);
      if (!project) return fail("invalid_argument", "Open a project first.");
      const r = JSON.parse(request) as {
        prompt: string;
        start?: number | null;
        duration?: number | null;
      };
      const audio = kind === "voice" || kind === "music" || kind === "sfx";
      const provider = audio ? "elevenlabs" : "fal";
      if (!genKeys.has(provider))
        return fail(
          "unsupported",
          `Add your ${provider === "fal" ? "fal.ai" : "ElevenLabs"} API key in Settings → Generation first.`,
        );
      const id = `g_${++seq}`;
      const itemId = `i_gen_${seq}`;
      const start = r.start ?? 0;
      fire(
        editProject(`generate ${kind}: placeholder`, (p) => {
          p.items.push(
            fromDetail({
              id: itemId,
              trackId: audio ? "t_audio" : "t_overlay",
              type: audio ? "audio" : kind === "video" ? "video" : "image",
              startTime: start,
              endTime: start + (r.duration ?? 5),
              name: r.prompt.slice(0, 40),
              pending: true,
              pendingDescription: r.prompt,
            }),
          );
        }),
      );
      const job = {
        id,
        kind,
        model: `${provider}:mock`,
        request: r,
        status: "running",
        progress: 0.1,
        error: null,
        estimateUsd: 0.025,
        createdAt: Date.now(),
      };
      genJobs.unshift(job);
      setTimeout(() => {
        job.status = "succeeded";
        job.progress = 1;
        fire(
          editProject(`generate ${kind}: ${r.prompt.slice(0, 40)}`, () => {
            const it = findItem(itemId);
            if (it) it.pending = false;
          }),
        );
        emit("generation-updated", JSON.stringify(job));
      }, control.generationMs ?? 400);
      return ok(JSON.stringify(job));
    },
    generationJobs: () => ok(JSON.stringify(genJobs)),
    cancelGeneration: (id: string) => {
      const j = genJobs.find((x) => x.id === id);
      if (j && j.status === "running") j.status = "cancelled";
      return ok(null);
    },
    listVoices: () =>
      genKeys.has("elevenlabs")
        ? ok(
            JSON.stringify([
              { id: "v_narrator", name: "Narrator" },
              { id: "v_bright", name: "Bright" },
            ]),
          )
        : fail(
            "invalid_argument",
            "Add your ElevenLabs API key in Settings → Generation.",
          ),
    reportUiState: (ui: { selection: string[]; viewer: string }) => (
      track(`reportUiState:${ui.viewer}:${ui.selection.length}`),
      ok(null)
    ),
    timelineFormats: () =>
      Promise.resolve(
        [
          ["fcpxml", "FCPXML (Final Cut Pro, DaVinci Resolve)"],
          ["xml", "XML (Premiere Pro, Final Cut Pro 7)"],
          ["edl", "EDL (CMX3600)"],
          ["otio", "OpenTimelineIO"],
        ].map(([id, label]) => ({ id, label, extension: id })),
      ),
    // A rough version of the core's report: item types timeline files can't hold, and what an EDL drops.
    exportTimeline: (
      format: string,
      fps: number | null,
      path: string | null,
      preview: boolean,
    ) => {
      track(`exportTimeline${preview ? "Report" : ""}:${format}@${fps}`);
      if (!project) return fail("invalid_argument", "Open a project first.");
      const omitted: { kind: string; count: number; reason: string }[] = [];
      const omit = (kind: string, count: number, reason: string) => {
        if (count) omitted.push({ kind, count, reason });
      };
      const items = project.items;
      const kinds: [string, string, string][] = [
        [
          "caption",
          "captions",
          "timeline files have no caption track; captions stay in the project",
        ],
        [
          "code",
          "motion graphics",
          "they are drawn by OpenVideoEditor and have no equivalent in other editors",
        ],
        [
          "shader",
          "shader layers",
          "they are drawn by OpenVideoEditor and have no equivalent in other editors",
        ],
        [
          "adjustment",
          "adjustment layers",
          "adjustment layers are not written to timeline files yet",
        ],
      ];
      for (const [kind, label, reason] of kinds)
        omit(label, items.filter((i) => i.kind === kind).length, reason);
      omit(
        "placeholders",
        items.filter((i) => i.pending).length,
        "they have no media yet",
      );
      if (format === "edl")
        omit(
          "titles",
          items.filter((i) => i.kind === "text").length,
          "an EDL has no titles",
        );
      const notCarried = items.some((i) => i.transitionIn || i.transitionOut)
        ? ["transitions"]
        : [];
      const report = { omitted, notCarried };
      if (preview) return ok({ path: null, report });
      return ok({
        path: path ?? `/mock/exports/${project.name}.${format}`,
        report,
      });
    },
    // Like the real import: tracks named "Imported …" above the existing ones, one clip per imported track here.
    importTimeline: (path: string | null, fps: number | null) => {
      track(`importTimeline:${path ?? "dialog"}@${fps}`);
      if (!project) return fail("invalid_argument", "Open a project first.");
      fire(
        editProject("import timeline", (p) => {
          const id = `t_imp_${++seq}`;
          p.tracks.unshift({
            id,
            name: "Imported V1",
            kind: "video",
            locked: false,
            visible: true,
            muted: false,
          });
          p.items.push({
            id: `i_imp_${seq}`,
            trackId: id,
            kind: "video",
            name: "Imported clip",
            start: 0,
            end: 4,
            trimStart: 0,
            mediaId: null,
            pending: true,
            hidden: false,
            muted: false,
            text: null,
            transitionIn: 0,
            transitionOut: 0,
            speed: 1,
          });
        }),
      );
      return ok({
        project: clone(project),
        tracks: 1,
        clips: 1,
        mediaAdded: 0,
        missing: ["Imported clip"],
      });
    },
    removeTrack: (id: string) =>
      editProject("remove track", (p) => {
        p.tracks = p.tracks.filter((t) => t.id !== id);
        p.items = p.items.filter((i) => i.trackId !== id);
      }),
    // Effects, transitions, layouts and colour (change 12). Previews are flat swatches here; the app renders them.
    fxCatalog: () => (track("fxCatalog"), ok(fxCatalogJson)),
    fxPreview: (kind: string, id: string) => {
      track(`fxPreview:${kind}:${id}`);
      let h = 0;
      for (const c of `${kind}${id}`) h = (h * 31 + c.charCodeAt(0)) >>> 0;
      const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90"><defs><linearGradient id="g"><stop offset="0" stop-color="hsl(${h % 360},70%,55%)"/><stop offset="1" stop-color="hsl(${(h >> 9) % 360},70%,35%)"/></linearGradient></defs><rect width="160" height="90" fill="url(#g)"/></svg>`;
      return ok(`data:image/svg+xml;base64,${btoa(svg)}`);
    },
    importLut: (path: string | null) => {
      track(`importLut:${path ?? "dialog"}`);
      const name = (path ?? "/mock/Imported look.cube")
        .split(/[\\/]/)
        .pop()!
        .replace(/[^A-Za-z0-9_-]+/g, "-")
        .replace(/-cube$/, "");
      return ok<string | null>(`luts/${name}.cube`);
    },
    matchColor: (
      itemId: string,
      referenceTime: number | null,
      strength: number | null,
    ) =>
      editProject("match colour", () => {
        track(`matchColor:${itemId}:${referenceTime ?? 0}:${strength ?? 1}`);
        const i = mock(itemId);
        if (!i) return;
        const g = (i.fields.colorGrade as Fields | undefined) ?? {};
        const { match: _match, ...rest } = g;
        void _match;
        const w = (r: number, gg: number, b: number) => ({ r, g: gg, b, y: 0 });
        i.fields.colorGrade = {
          ...rest,
          wheels: {
            lift: w(0, 0, 0),
            gamma: w(0, 0, 0),
            gain: w(0.08, 0.01, -0.06),
            offset: w(0.02, 0, -0.03),
          },
        };
      }),
    colorScopes: (time: number | null) => {
      track(`colorScopes:${time ?? 0}`);
      const bins = (shift: number) =>
        Array.from({ length: 64 }, (_, i) =>
          Math.exp(-(((i - 32 - shift) / 14) ** 2)),
        );
      return ok([bins(6), bins(0), bins(-6), bins(1)]);
    },
    agentStatus: () => ok(agentStatus()),
    agentSetKey: (key: string) => {
      if (!/^sk-ant-[\w-]{8,}$/.test(key.trim()))
        return fail(
          "invalid_argument",
          "That doesn't look like an Anthropic API key (they start with sk-ant-).",
        );
      control.agent.hasKey = true;
      return ok(agentStatus());
    },
    agentClearKey: () => {
      control.agent.hasKey = false;
      return ok(agentStatus());
    },
    agentSetModel: (model: string) => {
      if (!MODELS.some((m) => m.id === model))
        return fail("invalid_argument", "Unknown model.");
      agentModel = model;
      return ok(agentStatus());
    },
    agentChats: () =>
      ok(
        [...chats.entries()]
          .sort((a, b) => b[1].updatedAt - a[1].updatedAt)
          .map(([id, c]) => ({ id, title: c.title, updatedAt: c.updatedAt })),
      ),
    agentChat: (id: string) => {
      const c = chats.get(id);
      return c
        ? ok(JSON.parse(JSON.stringify(c.items)) as ChatItem_Serialize[])
        : fail("not_found", "That conversation no longer exists.");
    },
    agentSend: (chatId: string | null, text: string) => {
      if (!control.agent.hasKey)
        return fail(
          "invalid_argument",
          "Add an Anthropic API key to use the assistant.",
        );
      if (!project) return fail("invalid_argument", "Open a project first.");
      if (turn)
        return fail(
          "invalid_argument",
          "The assistant is still working on the last message.",
        );
      const id = chatId ?? `c_${++seq}`;
      if (!chats.has(id))
        chats.set(id, {
          title: text.split("\n")[0].slice(0, 60),
          updatedAt: Date.now(),
          items: [],
        });
      chats.get(id)!.items.push(item("user", text));
      turn = { stop: false };
      setTimeout(() => fire(runTurn(id)), 0);
      return ok({ chatId: id });
    },
    agentStop: () => {
      if (turn) {
        turn.stop = true;
        turn.answer?.("(stopped)");
      }
      return ok(null);
    },
    agentAnswer: (_questionId: string, answer: string) => {
      if (!turn?.answer)
        return fail("invalid_argument", "That question was already answered.");
      turn.answer(answer);
      turn.answer = undefined;
      return ok(null);
    },
    // Captions: simulated speech models, auto captions from a fixed transcript, and the style list.
    captionModels: () =>
      ok(
        CAPTION_MODELS.map(([id, label, sizeMb]) => ({
          id,
          label,
          sizeMb,
          installed: control.captions.installed.includes(id),
          default: id === "base",
        })),
      ),
    captionPresets: () => Promise.resolve(JSON.stringify(MOCK_CAPTION_PRESETS)),
    downloadCaptionModel: (model: string) => {
      track(`downloadCaptionModel:${model}`);
      if (captionJob)
        return fail(
          "invalid_argument",
          "Captions are already being made. Wait for them to finish or cancel.",
        );
      runCaptionJob(model, () =>
        emit("caption-job-progress", {
          stage: "done",
          fraction: 1,
          message: "",
          done: true,
          error: null,
          cancelled: false,
          captions: null,
          language: null,
          secondsTaken: null,
        }),
      );
      return ok(null);
    },
    autoCaption: (request: AutoCaptionRequest_Deserialize) => {
      track(
        `autoCaption:${request.model}:${request.language ?? "auto"}:${request.itemIds.length}`,
      );
      if (!project) return fail("invalid_argument", "Open a project first.");
      if (captionJob)
        return fail(
          "invalid_argument",
          "Captions are already being made. Wait for them to finish or cancel.",
        );
      const sources = (project.items as MockItem[]).filter(
        (i) =>
          (i.kind === "video" || i.kind === "audio") &&
          i.mediaId &&
          (request.itemIds.length
            ? request.itemIds.includes(i.id)
            : i.trackId === "t_main"),
      );
      if (sources.length === 0)
        return fail(
          "invalid_argument",
          "There is no clip with sound to caption. Add a video or audio clip first.",
        );
      runCaptionJob(request.model, () => {
        let n = 0;
        fire(
          editProject("auto captions", (p) => {
            const at = sources[0].start ?? 0;
            const track =
              p.tracks.find((t) => t.kind === "caption")?.id ?? "t_captions";
            if (request.replace)
              p.items = p.items.filter((i) => i.trackId !== track);
            for (const [k, line] of MOCK_TRANSCRIPT.entries()) {
              const words = line.split(" ");
              const start = at + k * 2;
              p.items.push(
                fromDetail({
                  id: `i${++seq}`,
                  trackId: track,
                  type: "caption",
                  name: "Caption",
                  startTime: start,
                  endTime: start + 1.9,
                  captionText: line,
                  captionPresetId: request.presetId ?? "butter-serif",
                  captionSegments: [
                    {
                      id: "s1",
                      text: line,
                      startTime: 0,
                      endTime: 1.9,
                      wordTimings: words.map((word, j) => ({
                        word,
                        start: j * 0.4,
                        end: j * 0.4 + 0.38,
                      })),
                    },
                  ],
                }),
              );
              n++;
            }
          }),
        );
        emit("caption-job-progress", {
          stage: "done",
          fraction: 1,
          message: "",
          done: true,
          error: null,
          cancelled: false,
          captions: n,
          language: request.language ?? "en",
          secondsTaken: 1.2,
        });
      });
      return ok(null);
    },
    cancelCaptions: () => {
      track("cancelCaptions");
      if (captionJob) {
        clearInterval(captionJob);
        captionJob = null;
        emit("caption-job-progress", {
          stage: "done",
          fraction: 0,
          message: "",
          done: true,
          error: null,
          cancelled: true,
          captions: null,
          language: null,
          secondsTaken: null,
        });
      }
      return ok(null);
    },
    applyCaptionStyle: (presetId: string, itemIds: string[]) => {
      track(`applyCaptionStyle:${presetId}:${itemIds.length}`);
      const targets = (project?.items ?? []).filter(
        (i) =>
          i.kind === "caption" &&
          (itemIds.length === 0 || itemIds.includes(i.id)),
      );
      if (project && targets.length === 0)
        return fail("invalid_argument", "There are no captions to style yet.");
      const label =
        MOCK_CAPTION_PRESETS.find((p) => p.id === presetId)?.label ?? presetId;
      return editProject(`caption style ${label.toLowerCase()}`, () => {
        for (const t of targets)
          applyEdit(t.id, { captionPresetId: presetId }, [
            "captionFontSize",
            "captionTextColor",
            "captionHighlightColor",
            "captionPosition",
          ]);
      });
    },
    // Motion graphics and fonts (see motionMock.ts).
    listFonts: () => ok(MOCK_FONTS),
    fontPreview: (family: string, text: string, size: number) =>
      ok(mockFontPreview(family, text, size)),
    listTemplates: () => ok(JSON.stringify(MOCK_TEMPLATES)),
    templateThumbnail: (id: string) => {
      const t = mockThumbnail(id);
      return t ? ok(t) : fail("not_found", "Unknown template.");
    },
    checkCode: (compositionJson: string) => ok(mockCheck(compositionJson)),
  };

  const makeEvent = (name: string) => ({
    listen: (cb: Listener) => {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name)!.add(cb);
      return Promise.resolve(() => listeners.get(name)?.delete(cb));
    },
  });
  const events = {
    agentUiEvent: makeEvent("agent-ui-event"),
    appErrorRaised: makeEvent("app-error-raised"),
    captionJobProgress: makeEvent("caption-job-progress"),
    exportProgress: makeEvent("export-progress"),
    openPathsRequested: makeEvent("open-paths-requested"),
    playbackChanged: makeEvent("playback-changed"),
    projectChanged: makeEvent("project-changed"),
    remoteOpened: makeEvent("remote-opened"),
    generationUpdated: makeEvent("generation-updated"),
    viewerBackendChanged: makeEvent("viewer-backend-changed"),
    viewerFrameReady: makeEvent("viewer-frame-ready"),
  } as unknown as Events;

  return { commands, events, control };
}

const mock = isTauri() ? null : createMock();
if (mock && typeof window !== "undefined") window.__oveMock = mock.control;

export const commands: Commands = mock ? mock.commands : generated.commands;
export const events: Events = mock ? mock.events : generated.events;
