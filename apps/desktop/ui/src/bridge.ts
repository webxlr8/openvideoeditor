// The UI's only door to the native core. Inside the app this is the generated bindings; in a
// plain browser (UI development, Playwright, unit tests) a mock with the same types stands in.

import { convertFileSrc } from "@tauri-apps/api/core";
import * as generated from "./bindings";
import type {
  AgentStatus_Serialize,
  AgentUiEvent_Serialize,
  AppError,
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
import { MOCK_FONTS, MOCK_TEMPLATES, mockCheck, mockFontPreview, mockThumbnail } from "./motionMock";

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
  rects: ViewerRect[];
  lastRect: ViewerRect | null;
  calls: string[];
  /** Simulates the native side routing paths to the window. */
  routePaths(paths: string[]): void;
  /** The simulated assistant: whether a key is stored and how long each streamed step takes. */
  agent: { hasKey: boolean; stepMs: number; ask: boolean };
}

declare global {
  interface Window {
    __oveMock?: MockControl;
  }
}

type Listener = (event: { payload: unknown }) => void;

function createMock(): { commands: Commands; events: Events; control: MockControl } {
  const listeners = new Map<string, Set<Listener>>();
  const emit = (name: string, payload: unknown) => listeners.get(name)?.forEach((cb) => cb({ payload }));
  const ok = <T>(data: T): Promise<Ok<T>> => Promise.resolve({ status: "ok", data });
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
      timer = setInterval(() => {
        const dir = Math.sign(state.speed ?? 0);
        const next = state.frame + dir;
        if (next < 0 || next > last()) {
          if (state.looping) state.frame = dir > 0 ? 0 : last();
          else setSpeed(0);
        } else state.frame = next;
        changed();
      }, 1000 / ((state.frameRate ?? 30) * Math.abs(state.speed ?? 1)));
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

  const control: MockControl = {
    rects: [],
    lastRect: null,
    calls: [],
    routePaths(paths) {
      emit("open-paths-requested", { paths });
    },
    agent: { hasKey: true, stepMs: 120, ask: false },
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
  const clone = (p: ProjectView_Serialize) => JSON.parse(JSON.stringify(p)) as ProjectView_Serialize;
  const withFlags = (p: ProjectView_Serialize): ProjectView_Serialize => ({
    ...p,
    canUndo: undoStack.length > 0,
    canRedo: redoStack.length > 0,
    durationSeconds: Math.max(0, ...p.items.map((i) => i.end ?? 0)),
  });
  // The timeline viewer follows edits, as the engine's timeline session does.
  let timelineOpen = false;
  const timelineFrames = () => Math.max(1, Math.ceil((project?.durationSeconds ?? 0) * 30));
  const publish = () => {
    if (!project) return ok<ProjectView_Serialize>(null as unknown as ProjectView_Serialize);
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
  const editProject = (summary: string, f: (p: ProjectView_Serialize) => void) => {
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
    ].map(([id, name, kind]) => ({ id, name, kind, locked: false, visible: true, muted: false })),
    items: [],
    media: [],
  });
  // Like the real import: unreadable files are skipped and reported through `appErrorRaised`.
  const importPaths = (all: string[]) => {
    const playable = /\.(mp4|m4v|mov|mxf|mkv|webm|m4a|wav|caf|opus|ogg|aac|png|jpe?g|webp)$/i;
    const paths = all.filter((p) => playable.test(p));
    const skipped = all.filter((p) => !playable.test(p));
    if (skipped.length > 0)
      emit("app-error-raised", {
        code: "unsupported",
        message: skipped.map((p) => `“${p.split(/[\\/]/).pop()}” can't be opened: this file type is not supported.`).join(" "),
      });
    if (paths.length === 0) return publish();
    return importValid(paths);
  };
  const importValid = (paths: string[]) =>
    editProject(`import ${paths.length} file${paths.length === 1 ? "" : "s"}`, (p) => {
      for (const path of paths) {
        const n = ++seq;
        const name = path.split(/[\\/]/).pop() ?? path;
        const audio = /\.(m4a|wav|caf|opus|ogg|aac)$/i.test(path);
        const track = audio ? "t_audio" : "t_main";
        const end = Math.max(0, ...p.items.filter((i) => i.trackId === track).map((i) => i.end ?? 0));
        p.media.push({ id: `m${n}`, name, path, kind: audio ? "audio" : "video", durationSeconds: 60, width: audio ? null : 1920, height: audio ? null : 1080, status: "online", thumbnail: null });
        p.items.push(fromDetail({ id: `i${n}`, trackId: track, type: audio ? "audio" : "video", name, startTime: end, endTime: end + 60, mediaItemId: `m${n}` }));
      }
    });
  const findItem = (id: string): ItemView_Serialize | undefined => project?.items.find((i) => i.id === id);

  // Full item fields (document names) ride along on each mock item, as the real core keeps them in the document.
  type Fields = Record<string, unknown>;
  type MockItem = ItemView_Serialize & { fields: Fields };
  const HEAD = ["id", "trackId", "type", "startTime", "endTime", "name", "trimStart", "mediaItemId", "pending"];
  const n = (v: unknown, d = 0) => (typeof v === "number" ? v : d);
  const refresh = (i: MockItem) => {
    const f = i.fields;
    const tr = (k: string) => n((f[k] as Fields | undefined)?.duration);
    i.hidden = f.hidden === true;
    i.muted = f.muted === true;
    i.text = typeof f.textContent === "string" ? f.textContent : typeof f.captionText === "string" ? f.captionText : null;
    i.transitionIn = tr("transitionIn");
    i.transitionOut = tr("transitionOut");
    i.speed = n(f.playbackSpeed, 1);
    return i;
  };
  function fromDetail(d: Fields): MockItem {
    const fields: Fields = {};
    for (const [k, v] of Object.entries(d)) if (!HEAD.includes(k)) fields[k] = v;
    return refresh({
      id: String(d.id ?? `i${++seq}`),
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
  const chats = new Map<string, { title: string; updatedAt: number; items: ChatItem_Serialize[] }>();
  let turn: { stop: boolean; answer?: (a: string) => void } | null = null;
  const agentStatus = (): AgentStatus_Serialize => ({
    hasKey: control.agent.hasKey,
    keySource: control.agent.hasKey ? "keychain" : null,
    model: agentModel,
    models: MODELS,
    running: turn !== null,
  });
  const agentEvent = (payload: AgentUiEvent_Serialize) => emit("agent-ui-event", payload);
  const wait = () => new Promise((r) => setTimeout(r, control.agent.stepMs));
  const item = (role: string, text: string, id: string | null = null, ok: boolean | null = null): ChatItem_Serialize => ({ role, text, id, ok, options: [] });
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
    const tool = async (id: string, name: string, label: string, run: () => string) => {
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
    await tool("tu_print", "print_timeline", "Read the timeline", () => `${project?.items.length ?? 0} items`);
    if (control.agent.ask && !t.stop) {
      const q = item("question", "Which style should the title use?", "tu_ask");
      q.options = ["Bold", "Minimal"];
      chat.items.push(q);
      agentEvent({ kind: "question", id: "tu_ask", question: q.text, options: q.options });
      const a = await new Promise<string>((r) => (t.answer = r));
      q.text += `\n→ ${a}`;
      q.ok = true;
    }
    await tool("tu_apply", "apply_timeline", "Edit the timeline", () => {
      void editProject("assistant: add opening title", (p) => {
        p.items.push(fromDetail({ id: `i${++seq}`, trackId: "t_text", type: "text", name: "Opening title", startTime: 0, endTime: 3, textContent: "Welcome" }));
      });
      return "added 1";
    });
    await tool("tu_verify", "verify_timeline", "Check the timeline", () => "no problems");
    await say("Done: I added a 3-second **Welcome** title at the start. Undo removes it in one step.");
    chat.updatedAt = Date.now();
    turn = null;
    agentEvent({ kind: "turnDone", chatId, stopped: t.stop, durationMs: Date.now() - started, inputTokens: 5400, outputTokens: 320, cacheReadTokens: 4800, costUsd: 0.0148, note: null });
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
      }),
    takePendingOpenPaths: () => (track("takePendingOpenPaths"), ok<string[]>([])),
    openMedia: (path: string) => {
      track(`openMedia:${path}`);
      if (!/\.(mp4|m4v|mov|mxf|mkv|webm|m4a|wav|caf|opus|ogg|aac)$/i.test(path))
        return fail("unsupported", `“${path.split(/[\\/]/).pop()}” can't be opened: this file type is not supported.`);
      const info = media(path);
      timelineOpen = false;
      setSpeed(0);
      Object.assign(state, { hasMedia: true, frame: 0, totalFrames: info.totalFrames, frameRate: 30 });
      changed();
      return ok(info);
    },
    openMediaDialog: () => commands.openMedia("/mock/Sample clip.mov") as ReturnType<Commands["openMediaDialog"]>,
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
    debugPanicWorker: () => fail("worker_panic", "An internal error stopped a test worker. Details are in the log."),
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
    importMediaDialog: () => importPaths(["/mock/Sample clip.mov"]) as ReturnType<Commands["importMediaDialog"]>,
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
      editProject(`delete ${ids.length} item${ids.length === 1 ? "" : "s"}`, (p) => {
        p.items = p.items.filter((i) => !ids.includes(i.id));
      }),
    splitItem: (id: string, at: number | null) =>
      editProject("split clip", (p) => {
        const i = findItem(id);
        if (!i || at === null || at <= (i.start ?? 0) || at >= (i.end ?? 0)) return;
        p.items.push({ ...i, id: `${i.id}_b`, start: at, trimStart: (i.trimStart ?? 0) + at - (i.start ?? 0) });
        i.end = at;
      }),
    setAspectRatio: (aspect: string) => editProject("change aspect ratio", (p) => void (p.aspectRatio = aspect)),
    renameProject: (name: string) => editProject("rename project", (p) => void (p.name = name)),
    setTrackFlag: (id: string, flag: string, value: boolean) =>
      editProject("track setting", (p) => {
        const t = p.tracks.find((x) => x.id === id);
        if (t && (flag === "muted" || flag === "visible" || flag === "locked")) t[flag] = value;
      }),
    undo: () => {
      if (!project || undoStack.length === 0) return fail("invalid_argument", "Nothing to undo.");
      redoStack.push(clone(project));
      project = undoStack.pop()!;
      return publish();
    },
    redo: () => {
      if (!project || redoStack.length === 0) return fail("invalid_argument", "Nothing to redo.");
      undoStack.push(clone(project));
      project = redoStack.pop()!;
      return publish();
    },
    itemDetail: (id: string) => {
      const i = mock(id);
      return i ? ok(JSON.stringify(toDetail(i))) : fail("not_found", "That item no longer exists.");
    },
    updateItem: (id: string, setJson: string, unset: string[], summary: string | null) =>
      editProject(summary ?? "edit item", () => applyEdit(id, JSON.parse(setJson) as Fields, unset)),
    updateItems: (editsJson: string, summary: string | null) =>
      editProject(summary ?? "edit items", () => {
        for (const e of JSON.parse(editsJson) as { id: string; set?: Fields; unset?: string[] }[]) applyEdit(e.id, e.set ?? {}, e.unset ?? []);
      }),
    addItems: (itemsJson: string, summary: string | null) =>
      editProject(summary ?? "add items", (p) => {
        for (const d of JSON.parse(itemsJson) as Fields[]) p.items.push(fromDetail({ ...d, id: d.id ?? `i${++seq}` }));
      }),
    duplicateItems: (ids: string[]) =>
      editProject("duplicate", (p) => {
        for (const id of ids) {
          const i = mock(id);
          if (!i) continue;
          const len = (i.end ?? 0) - (i.start ?? 0);
          p.items.push(fromDetail({ ...toDetail(i), id: `i${++seq}`, startTime: i.end, endTime: (i.end ?? 0) + len }));
        }
      }),
    pasteItems: (itemsJson: string, at: number | null) =>
      editProject("paste", (p) => {
        const list = JSON.parse(itemsJson) as Fields[];
        const min = Math.min(...list.map((d) => n(d.startTime)));
        for (const d of list)
          p.items.push(fromDetail({ ...d, id: `i${++seq}`, startTime: (at ?? 0) + n(d.startTime) - min, endTime: (at ?? 0) + n(d.endTime) - min }));
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
        const t = { id: `t${++seq}`, name, kind, locked: false, visible: true, muted: false };
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
        p.tracks = order.map((id) => p.tracks.find((t) => t.id === id)!).filter(Boolean);
      }),
    openTimeline: () => {
      track("openTimeline");
      if (!project) return fail("invalid_argument", "No project is open.");
      const [width, height] = project.aspectRatio === "9:16" ? [1080, 1920] : project.aspectRatio === "1:1" ? [1080, 1080] : [1920, 1080];
      timelineOpen = true;
      setSpeed(0);
      Object.assign(state, { hasMedia: true, frame: 0, totalFrames: timelineFrames(), frameRate: 30 });
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
      if (exportTimer) return fail("invalid_argument", "An export is already running.");
      const total = Math.max(1, Math.round((project.durationSeconds ?? 0) * request.fps));
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
        emit("export-progress", { fraction: null, frame: 0, totalFrames: 0, done: true, error: "Export cancelled.", path: null, encoder: null, secondsTaken: null });
      }
      return ok(null);
    },
    chooseExportPath: (defaultName: string) => (track("chooseExportPath"), ok<string | null>(`/mock/exports/${defaultName}.mp4`)),
    revealPath: (path: string) => (track(`revealPath:${path}`), ok(null)),
    removeTrack: (id: string) =>
      editProject("remove track", (p) => {
        p.tracks = p.tracks.filter((t) => t.id !== id);
        p.items = p.items.filter((i) => i.trackId !== id);
      }),
    agentStatus: () => ok(agentStatus()),
    agentSetKey: (key: string) => {
      if (!/^sk-ant-[\w-]{8,}$/.test(key.trim())) return fail("invalid_argument", "That doesn't look like an Anthropic API key (they start with sk-ant-).");
      control.agent.hasKey = true;
      return ok(agentStatus());
    },
    agentClearKey: () => {
      control.agent.hasKey = false;
      return ok(agentStatus());
    },
    agentSetModel: (model: string) => {
      if (!MODELS.some((m) => m.id === model)) return fail("invalid_argument", "Unknown model.");
      agentModel = model;
      return ok(agentStatus());
    },
    agentChats: () =>
      ok([...chats.entries()].sort((a, b) => b[1].updatedAt - a[1].updatedAt).map(([id, c]) => ({ id, title: c.title, updatedAt: c.updatedAt }))),
    agentChat: (id: string) => {
      const c = chats.get(id);
      return c ? ok(JSON.parse(JSON.stringify(c.items)) as ChatItem_Serialize[]) : fail("not_found", "That conversation no longer exists.");
    },
    agentSend: (chatId: string | null, text: string) => {
      if (!control.agent.hasKey) return fail("invalid_argument", "Add an Anthropic API key to use the assistant.");
      if (!project) return fail("invalid_argument", "Open a project first.");
      if (turn) return fail("invalid_argument", "The assistant is still working on the last message.");
      const id = chatId ?? `c_${++seq}`;
      if (!chats.has(id)) chats.set(id, { title: text.split("\n")[0].slice(0, 60), updatedAt: Date.now(), items: [] });
      chats.get(id)!.items.push(item("user", text));
      turn = { stop: false };
      setTimeout(() => void runTurn(id), 0);
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
      if (!turn?.answer) return fail("invalid_argument", "That question was already answered.");
      turn.answer(answer);
      turn.answer = undefined;
      return ok(null);
    },
    // Motion graphics and fonts (see motionMock.ts).
    listFonts: () => ok(MOCK_FONTS),
    fontPreview: (family: string, text: string, size: number) => ok(mockFontPreview(family, text, size)),
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
    exportProgress: makeEvent("export-progress"),
    openPathsRequested: makeEvent("open-paths-requested"),
    playbackChanged: makeEvent("playback-changed"),
    projectChanged: makeEvent("project-changed"),
    viewerBackendChanged: makeEvent("viewer-backend-changed"),
    viewerFrameReady: makeEvent("viewer-frame-ready"),
  } as unknown as Events;

  return { commands, events, control };
}

const mock = isTauri() ? null : createMock();
if (mock && typeof window !== "undefined") window.__oveMock = mock.control;

export const commands: Commands = mock ? mock.commands : generated.commands;
export const events: Events = mock ? mock.events : generated.events;
