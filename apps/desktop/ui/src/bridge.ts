// The UI's only door to the native core. Inside the app this is the generated bindings; in a
// plain browser (UI development, Playwright, unit tests) a mock with the same types stands in.

import { convertFileSrc } from "@tauri-apps/api/core";
import * as generated from "./bindings";
import type {
  AppError,
  AppInfo_Serialize,
  Diagnostics_Serialize,
  MediaInfo_Serialize,
  PlaybackState,
  TransportCommand,
  ViewerBackend,
  ViewerRect,
} from "./bindings";

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
      setSpeed(0);
      Object.assign(state, { hasMedia: true, frame: 0, totalFrames: info.totalFrames, frameRate: 30 });
      changed();
      return ok(info);
    },
    openMediaDialog: () => commands.openMedia("/mock/Sample clip.mov") as ReturnType<Commands["openMediaDialog"]>,
    closeMedia: () => {
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
  };

  const makeEvent = (name: string) => ({
    listen: (cb: Listener) => {
      if (!listeners.has(name)) listeners.set(name, new Set());
      listeners.get(name)!.add(cb);
      return Promise.resolve(() => listeners.get(name)?.delete(cb));
    },
  });
  const events = {
    appErrorRaised: makeEvent("app-error-raised"),
    openPathsRequested: makeEvent("open-paths-requested"),
    playbackChanged: makeEvent("playback-changed"),
    viewerBackendChanged: makeEvent("viewer-backend-changed"),
    viewerFrameReady: makeEvent("viewer-frame-ready"),
  } as unknown as Events;

  return { commands, events, control };
}

const mock = isTauri() ? null : createMock();
if (mock && typeof window !== "undefined") window.__oveMock = mock.control;

export const commands: Commands = mock ? mock.commands : generated.commands;
export const events: Events = mock ? mock.events : generated.events;
