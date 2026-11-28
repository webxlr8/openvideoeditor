// The Audio panel's Duck popover: lower the music under the dialogue. The result is volume keyframes on the music
// clips (visible on their gain lines, editable, one undo step). Preview says what would change first.

import { useState } from "react";
import "./audio.css";
import type { DuckPreview, DuckRequest, ProjectView_Serialize } from "./bindings";
import { commands } from "./bridge";
import type { EditResult } from "./edit";

export interface DuckPopoverProps {
  project: ProjectView_Serialize;
  onEdit: (p: EditResult) => void;
  onClose: () => void;
}

/** Tracks that hold clips with sound, for the two pickers. */
function soundTracks(p: ProjectView_Serialize) {
  return p.tracks.filter((t) => p.items.some((i) => i.trackId === t.id && (i.kind === "audio" || i.kind === "video") && i.mediaId));
}

export function DuckPopover({ project, onEdit, onClose }: DuckPopoverProps) {
  const tracks = soundTracks(project);
  const firstAudio = tracks.find((t) => t.kind === "audio")?.id ?? tracks[0]?.id ?? "";
  const firstOther = tracks.find((t) => t.id !== firstAudio)?.id ?? "";
  const [music, setMusic] = useState(firstAudio);
  const [under, setUnder] = useState(firstOther);
  const [depth, setDepth] = useState(-12);
  const [attack, setAttack] = useState(0.2);
  const [release, setRelease] = useState(0.4);
  const [hold, setHold] = useState(0.5);
  const [source, setSource] = useState("auto");
  const [preview, setPreview] = useState<DuckPreview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const request = (): DuckRequest => ({ music: [music], under: [under], depth, attack, release, hold, source, mode: "replace" });
  const ready = !!music && !!under && music !== under;

  const runPreview = async () => {
    setError(null);
    setPreview(null);
    const r = await commands.duckPreview(request());
    if (r.status === "ok") setPreview(r.data);
    else setError(r.error.message);
  };

  const duck = async () => {
    setError(null);
    const p = commands.duckAudio(request());
    onEdit(p);
    const r = await p;
    if (r.status === "ok") onClose();
    else setError(r.error.message);
  };

  const num = (v: string, d: number) => {
    const n = Number(v);
    return Number.isFinite(n) ? Math.min(10, Math.max(0, n)) : d;
  };

  return (
    <div className="duck-popover" data-testid="duck-popover" role="dialog" aria-label="Duck music under dialogue">
      <label className="duck-row">
        <span>Lower</span>
        <select value={music} onChange={(e) => setMusic(e.target.value)} data-testid="duck-music">
          {tracks.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </label>
      <label className="duck-row">
        <span>Under</span>
        <select value={under} onChange={(e) => setUnder(e.target.value)} data-testid="duck-under">
          <option value="">Choose dialogue…</option>
          {tracks.map((t) => (
            <option key={t.id} value={t.id}>
              {t.name}
            </option>
          ))}
        </select>
      </label>
      <label className="duck-row">
        <span>By</span>
        <input type="range" min={-30} max={-3} step={1} value={depth} onChange={(e) => setDepth(Number(e.target.value))} data-testid="duck-depth" />
        <output data-testid="duck-depth-value">{depth} dB</output>
      </label>
      <div className="duck-row duck-times">
        <label>
          Attack <input type="number" min={0} max={10} step={0.05} value={attack} onChange={(e) => setAttack(num(e.target.value, 0.2))} data-testid="duck-attack" /> s
        </label>
        <label>
          Release <input type="number" min={0} max={10} step={0.05} value={release} onChange={(e) => setRelease(num(e.target.value, 0.4))} data-testid="duck-release" /> s
        </label>
        <label>
          Hold <input type="number" min={0} max={10} step={0.1} value={hold} onChange={(e) => setHold(num(e.target.value, 0.5))} data-testid="duck-hold" /> s
        </label>
      </div>
      <label className="duck-row">
        <span>Speech</span>
        <select value={source} onChange={(e) => setSource(e.target.value)} data-testid="duck-source">
          <option value="auto">Transcript if there is one, else the level</option>
          <option value="transcript">From the transcript</option>
          <option value="level">From the level</option>
        </select>
      </label>
      {preview && (
        <p className="small" data-testid="duck-preview">
          Found {preview.lines} {preview.lines === 1 ? "line" : "lines"} of speech (from the {preview.source}). Adds {preview.keyframes} volume
          keyframes to {preview.musicItems} music {preview.musicItems === 1 ? "clip" : "clips"}.
        </p>
      )}
      {error && (
        <p className="small error-text" role="alert" data-testid="duck-error">
          {error}
        </p>
      )}
      <div className="duck-actions">
        <button className="chip-sm" onClick={onClose}>
          Cancel
        </button>
        <button className="chip-sm" disabled={!ready} onClick={() => {
            runPreview().catch(() => undefined);
          }} data-testid="duck-preview-run">
          Preview
        </button>
        <button className="chip-sm primary" disabled={!ready} onClick={() => {
            duck().catch(() => undefined);
          }} data-testid="duck-apply">
          Duck
        </button>
      </div>
    </div>
  );
}
