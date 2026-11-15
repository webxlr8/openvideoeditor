// Generate panel (change 14): images, video, voice, music and sound effects with the person's own provider keys.
// The estimate is shown before anything is spent; generating needs a second, explicit confirmation. A pending
// placeholder appears on the timeline at once and is filled when the job finishes.

import { useCallback, useEffect, useMemo, useState } from "react";
import { commands, events } from "./bridge";
import type { AppError, KeyCheck } from "./bindings";
import { Icon } from "./icons";
import { fire, reportError, runCommand, unlisten } from "./notify";
import { PanelHint } from "./onboarding/PanelHint";
import type { PanelProps } from "./Panels";

export type GenKind = "image" | "video" | "voice" | "music" | "sfx";

export interface GenModel {
  id: string;
  provider: string;
  kind: GenKind;
  label: string;
  durations: number[];
  maxSeconds: number;
  aspects: string[];
  default: boolean;
  price: { per: string; usd?: number };
}

export interface KeyStatus {
  provider: string;
  label: string;
  keyUrl: string;
  envVar: string;
  hasKey: boolean;
  source: string | null;
}

export interface GenJob {
  id: string;
  kind: GenKind;
  model: string;
  request: { prompt: string };
  status: "queued" | "running" | "succeeded" | "failed" | "cancelled";
  progress: number | null;
  error: string | null;
  estimateUsd: number | null;
  createdAt: number;
}

interface Plan {
  estimateUsd: number | null;
  seconds: number;
  trackId: string;
  start: number;
}

export interface Catalog {
  models: GenModel[];
  keys: KeyStatus[];
}

const KINDS: { id: GenKind; label: string; hint: string }[] = [
  {
    id: "image",
    label: "Image",
    hint: "A still, placed as a clip on the Overlay track.",
  },
  {
    id: "video",
    label: "Video",
    hint: "A short shot from a description, or from a first frame.",
  },
  {
    id: "voice",
    label: "Voice",
    hint: "Narration from text, on the Audio track.",
  },
  { id: "music", label: "Music", hint: "A music bed from a description." },
  { id: "sfx", label: "Sound", hint: "A sound effect." },
];

const PLACEHOLDER: Record<GenKind, string> = {
  image: "A lighthouse on a cliff at dawn, soft mist, wide shot",
  video: "Slow push-in on waves breaking against a harbour wall at sunrise",
  voice: "Every morning, the harbour wakes before the town does.",
  music: "Calm ambient piano with soft strings, 70 bpm, hopeful",
  sfx: "Seagulls and distant waves",
};

export function usd(v: number | null | undefined) {
  if (v == null) return "cost unknown";
  return v < 0.1 ? `≈ $${v.toFixed(3)}` : `≈ $${v.toFixed(2)}`;
}

export function priceLabel(m: GenModel) {
  const p = m.price;
  if (p.usd == null) return "";
  if (p.per === "image") return `$${p.usd}/image`;
  if (p.per === "second") return `$${p.usd}/s`;
  if (p.per === "kiloChar") return `$${p.usd}/1k chars`;
  return "";
}

export async function loadCatalog(): Promise<Catalog> {
  try {
    return JSON.parse(await commands.generationCatalog()) as Catalog;
  } catch (e) {
    reportError(e);
    return { models: [], keys: [] };
  }
}

/** Inline key entry for one provider (also used in Settings). */
export function KeyEntry({
  status,
  onSaved,
  onError,
}: {
  status: KeyStatus;
  onSaved: (c: Catalog) => void;
  onError: (e: AppError) => void;
}) {
  const [key, setKey] = useState("");
  const [check, setCheck] = useState<KeyCheck | "checking" | null>(null);
  // fal.ai has no free request to try a key with; it is checked on first use.
  const testable = status.provider !== "fal";
  const test = () => {
    setCheck("checking");
    runCommand(commands.testProviderKey(status.provider), setCheck, (e) => {
      setCheck(null);
      onError(e);
    });
  };
  const save = async () => {
    const r = await commands.setProviderKey(status.provider, key);
    if (r.status === "error") return onError(r.error);
    setKey("");
    onSaved(JSON.parse(r.data) as Catalog);
    if (testable) test();
  };
  const clear = async () => {
    const r = await commands.clearProviderKey(status.provider);
    if (r.status === "error") return onError(r.error);
    setCheck(null);
    onSaved(JSON.parse(r.data) as Catalog);
  };
  return (
    <div className="gen-key" data-testid={`key-${status.provider}`}>
      <div className="gen-key-head">
        <strong>{status.label}</strong>
        {status.hasKey ? (
          <span className="badge ok">
            {status.source === "environment"
              ? `from ${status.envVar}`
              : "key saved"}
          </span>
        ) : (
          <span className="badge">no key</span>
        )}
      </div>
      <div className="gen-key-row">
        <input
          type="password"
          placeholder={
            status.hasKey ? "Replace the key…" : "Paste your API key"
          }
          value={key}
          onChange={(e) => setKey(e.target.value)}
          aria-label={`${status.label} API key`}
        />
        <button
          className="btn"
          disabled={!key.trim()}
          onClick={() => fire(save(), onError)}
        >
          Save
        </button>
        {status.hasKey && testable && (
          <button
            className="btn"
            disabled={check === "checking"}
            onClick={test}
            data-testid={`key-test-${status.provider}`}
          >
            Test
          </button>
        )}
        {status.hasKey && status.source !== "environment" && (
          <button className="btn" onClick={() => fire(clear(), onError)}>
            Remove
          </button>
        )}
      </div>
      {check && (
        <p
          className={
            check === "checking"
              ? "muted small"
              : check.ok
                ? "key-ok small"
                : "key-error small"
          }
          role="status"
          data-testid={`key-check-${status.provider}`}
        >
          {check === "checking" ? "Checking the key…" : check.message}
        </p>
      )}
      <p className="muted small">
        Kept in your system's credential store; sent only to {status.label}.
        Create one at <span className="mono">{status.keyUrl}</span>.
      </p>
    </div>
  );
}

function JobRow({ job }: { job: GenJob }) {
  const running = job.status === "queued" || job.status === "running";
  return (
    <li className={`gen-job ${job.status}`} data-testid="gen-job">
      <Icon
        name={job.kind === "image" || job.kind === "video" ? "media" : "audio"}
        size={14}
      />
      <div className="gen-job-text">
        <span className="gen-job-prompt" title={job.request.prompt}>
          {job.request.prompt}
        </span>
        <span className="muted small">
          {job.kind} · {job.status === "succeeded" ? "done" : job.status}
          {running &&
            job.progress != null &&
            ` ${Math.round(job.progress * 100)}%`}
          {job.estimateUsd != null && ` · ${usd(job.estimateUsd)}`}
          {job.error && ` · ${job.error}`}
        </span>
      </div>
      {running && (
        <button
          className="icon-btn"
          title="Cancel"
          aria-label="Cancel"
          onClick={() => runCommand(commands.cancelGeneration(job.id))}
        >
          <Icon name="close" size={13} />
        </button>
      )}
    </li>
  );
}

export function GeneratePanel({ project, playhead, onError }: PanelProps) {
  const fail = useCallback((e: AppError) => onError?.(e), [onError]);
  const [catalog, setCatalog] = useState<Catalog>({ models: [], keys: [] });
  const [kind, setKind] = useState<GenKind>("image");
  const [prompt, setPrompt] = useState("");
  const [modelId, setModelId] = useState<string>("");
  const [duration, setDuration] = useState<number | null>(null);
  const [aspect, setAspect] = useState<string>(project.aspectRatio);
  const [voice, setVoice] = useState("");
  const [voices, setVoices] = useState<{ id: string; name: string }[]>([]);
  const [atPlayhead, setAtPlayhead] = useState(true);
  const [jobs, setJobs] = useState<GenJob[]>([]);

  useEffect(() => fire(loadCatalog().then(setCatalog)), []);
  const models = useMemo(
    () => catalog.models.filter((m) => m.kind === kind),
    [catalog, kind],
  );
  const model =
    models.find((m) => m.id === modelId) ??
    models.find((m) => m.default) ??
    models[0];
  const key = catalog.keys.find((k) => k.provider === model?.provider);

  const refreshJobs = useCallback(
    () =>
      runCommand(
        commands.generationJobs(),
        (json) => setJobs(JSON.parse(json) as GenJob[]),
        fail,
      ),
    [fail],
  );
  useEffect(() => {
    let live = true;
    runCommand(
      commands.generationJobs(),
      (json) => {
        if (live) setJobs(JSON.parse(json) as GenJob[]);
      },
      fail,
    );
    const un = events.generationUpdated.listen(refreshJobs);
    return () => {
      live = false;
      unlisten(un);
    };
  }, [refreshJobs, fail, project.id]);
  const anyRunning = jobs.some(
    (j) => j.status === "queued" || j.status === "running",
  );
  useEffect(() => {
    if (!anyRunning) return;
    const t = setInterval(refreshJobs, 1500);
    return () => clearInterval(t);
  }, [anyRunning, refreshJobs]);

  const chooseKind = (k: GenKind) => {
    setKind(k);
    setModelId("");
    setDuration(null);
  };

  const request = useMemo(
    () => ({
      prompt: prompt.trim(),
      model: model?.id ?? null,
      duration: duration ?? (kind === "music" ? 30 : kind === "sfx" ? 5 : null),
      aspect: kind === "image" || kind === "video" ? aspect : null,
      voice: kind === "voice" && voice ? voice : null,
      start: atPlayhead ? Math.round(playhead * 1000) / 1000 : null,
    }),
    [prompt, model, duration, kind, aspect, voice, atPlayhead, playhead],
  );

  // Live plan and estimate for the current request (stale answers are ignored by key).
  const reqKey = `${kind}|${JSON.stringify(request)}`;
  const [planned, setPlanned] = useState<{
    key: string;
    plan: Plan | null;
    error: string | null;
  }>({ key: "", plan: null, error: null });
  useEffect(() => {
    if (!request.prompt || !model) return;
    const t = setTimeout(
      () =>
        runCommand(
          commands.planGeneration(kind, JSON.stringify(request)),
          (json) =>
            setPlanned({
              key: reqKey,
              plan: JSON.parse(json) as Plan,
              error: null,
            }),
          (e) => setPlanned({ key: reqKey, plan: null, error: e.message }),
        ),
      250,
    );
    return () => clearTimeout(t);
  }, [kind, request, model, reqKey]);
  const fresh = planned.key === reqKey && !!request.prompt;
  const plan = fresh ? planned.plan : null;
  const planError = fresh ? planned.error : null;
  const [confirmFor, setConfirmFor] = useState<string | null>(null);
  const confirming = confirmFor === reqKey;
  const setConfirming = (on: boolean) => setConfirmFor(on ? reqKey : null);

  const start = async () => {
    if (!confirming) return setConfirming(true);
    setConfirming(false);
    const r = await commands.startGeneration(kind, JSON.stringify(request));
    if (r.status === "error") return fail(r.error);
    setPrompt("");
    refreshJobs();
  };

  const loadVoices = async () => {
    const r = await commands.listVoices();
    if (r.status === "error") return fail(r.error);
    setVoices(JSON.parse(r.data) as { id: string; name: string }[]);
  };

  return (
    <div className="gen-panel" data-testid="generate-panel">
      <div className="xp-seg gen-kinds" role="tablist">
        {KINDS.map((k) => (
          <button
            key={k.id}
            role="tab"
            aria-selected={kind === k.id}
            className={kind === k.id ? "on" : ""}
            onClick={() => chooseKind(k.id)}
            data-testid={`gen-kind-${k.id}`}
          >
            {k.label}
          </button>
        ))}
      </div>
      {jobs.length === 0 && !prompt && (
        <PanelHint
          panel="generate"
          action="Try an example"
          onAction={() => setPrompt(PLACEHOLDER[kind])}
        >
          Generate makes pictures, video, voice and music from a description, with your own provider keys.
        </PanelHint>
      )}
      <p className="muted small">{KINDS.find((k) => k.id === kind)?.hint}</p>
      {key && !key.hasKey && (
        <KeyEntry status={key} onSaved={setCatalog} onError={fail} />
      )}
      <textarea
        className="gen-prompt"
        rows={kind === "voice" ? 5 : 3}
        placeholder={PLACEHOLDER[kind]}
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
        data-testid="gen-prompt"
      />
      <div className="gen-grid">
        <label>
          <span>Model</span>
          <select
            value={model?.id ?? ""}
            onChange={(e) => setModelId(e.target.value)}
            data-testid="gen-model"
          >
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label} {priceLabel(m) && `(${priceLabel(m)})`}
              </option>
            ))}
          </select>
        </label>
        {(kind === "image" || kind === "video") &&
          model &&
          model.aspects.length > 0 && (
            <label>
              <span>Aspect</span>
              <select
                value={aspect}
                onChange={(e) => setAspect(e.target.value)}
              >
                {[...new Set([project.aspectRatio, ...model.aspects])].map(
                  (a) => (
                    <option key={a} value={a}>
                      {a}
                    </option>
                  ),
                )}
              </select>
            </label>
          )}
        {kind === "video" && model && model.durations.length > 0 && (
          <label>
            <span>Length</span>
            <select
              value={duration ?? model.durations[0]}
              onChange={(e) => setDuration(Number(e.target.value))}
            >
              {model.durations.map((d) => (
                <option key={d} value={d}>
                  {d} s
                </option>
              ))}
            </select>
          </label>
        )}
        {(kind === "music" || kind === "sfx") && (
          <label>
            <span>Length (s)</span>
            <input
              type="number"
              min={kind === "music" ? 3 : 0.5}
              max={kind === "music" ? 300 : 30}
              step={kind === "music" ? 1 : 0.5}
              value={duration ?? (kind === "music" ? 30 : 5)}
              onChange={(e) => setDuration(Number(e.target.value))}
            />
          </label>
        )}
        {kind === "voice" && (
          <label>
            <span>Voice</span>
            {voices.length > 0 ? (
              <select value={voice} onChange={(e) => setVoice(e.target.value)}>
                <option value="">Default</option>
                {voices.map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.name}
                  </option>
                ))}
              </select>
            ) : (
              <div className="gen-key-row">
                <input
                  placeholder="Default"
                  value={voice}
                  onChange={(e) => setVoice(e.target.value)}
                />
                <button
                  className="btn"
                  disabled={!key?.hasKey}
                  onClick={() => fire(loadVoices(), fail)}
                >
                  List
                </button>
              </div>
            )}
          </label>
        )}
      </div>
      <label className="gen-check">
        <input
          type="checkbox"
          checked={atPlayhead}
          onChange={(e) => setAtPlayhead(e.target.checked)}
        />{" "}
        Place at the playhead ({playhead.toFixed(2)} s); otherwise after the
        last clip
      </label>
      {planError && <p className="xp-error small">{planError}</p>}
      {plan && (
        <p className="gen-estimate" data-testid="gen-estimate">
          {usd(plan.estimateUsd)} · {plan.seconds.toFixed(1)} s on{" "}
          {project.tracks.find((t) => t.id === plan.trackId)?.name ??
            plan.trackId}{" "}
          · billed to your {key?.label ?? "provider"} account
        </p>
      )}
      <div className="xp-actions">
        {confirming && (
          <button className="btn" onClick={() => setConfirming(false)}>
            Cancel
          </button>
        )}
        <button
          className={`btn primary${confirming ? " warn" : ""}`}
          disabled={!plan || !key?.hasKey}
          onClick={() => fire(start(), fail)}
          data-testid="gen-start"
        >
          {confirming ? `Confirm: spend ${usd(plan?.estimateUsd)}` : "Generate"}
        </button>
      </div>
      {jobs.length > 0 && (
        <>
          <h4 className="gen-jobs-title">Jobs</h4>
          <ul className="gen-jobs">
            {jobs.slice(0, 12).map((j) => (
              <JobRow key={j.id} job={j} />
            ))}
          </ul>
        </>
      )}
    </div>
  );
}
