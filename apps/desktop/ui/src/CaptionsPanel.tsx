// Captions panel: auto-captions from clip sound (offline speech recognition, model downloaded on first use), the
// caption style gallery (apply to the selected captions or all of them), and the transcript editor. Every change is
// one revision.

import { useCallback, useEffect, useMemo, useState } from "react";
import type { CaptionJobProgress, CaptionModel, ItemView_Serialize } from "./bindings";
import { commands, events } from "./bridge";
import { CAPTION_LANGUAGES, CaptionSample, type CaptionSegment, DEFAULT_CAPTION_PRESET, animationLabel, retimeSegment, useCaptionPresets } from "./CaptionsPanelStyles";
import { addItems, firstTrack, itemDetail, snapToFrame, updateItem } from "./edit";
import type { PanelProps } from "./Panels";
import { Icon } from "./icons";
import { fire, runCommand, unlisten } from "./notify";
import "./CaptionsPanel.css";

type Job = { stage: string; fraction: number; message: string } | null;

const fmt = (t: number) => {
  const m = Math.floor(t / 60);
  const s = t - m * 60;
  return `${m}:${s.toFixed(1).padStart(4, "0")}`;
};

export function CaptionsPanel({ project, playhead, selection, onEdit, onSelect, onError }: PanelProps) {
  const presets = useCaptionPresets();
  const [models, setModels] = useState<CaptionModel[]>([]);
  const [model, setModel] = useState("base");
  const [language, setLanguage] = useState("auto");
  const [style, setStyle] = useState(DEFAULT_CAPTION_PRESET);
  const [replace, setReplace] = useState(true);
  const [useSelection, setUseSelection] = useState(false);
  const [job, setJob] = useState<Job>(null);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [styleScope, setStyleScope] = useState<"auto" | "all">("auto");

  const refreshModels = useCallback(() => runCommand(commands.captionModels(), setModels, onError), [onError]);
  useEffect(refreshModels, [refreshModels]);

  useEffect(() => {
    const off = events.captionJobProgress.listen((e) => {
      const p = e.payload as CaptionJobProgress;
      if (!p.done) {
        setJob({ stage: p.stage, fraction: p.fraction ?? 0, message: p.message });
        return;
      }
      setJob(null);
      refreshModels();
      if (p.cancelled) setResult({ ok: false, text: "Stopped." });
      else if (p.error) setResult({ ok: false, text: p.error });
      else if (p.captions != null)
        setResult({
          ok: p.captions > 0,
          text: p.captions > 0 ? `Added ${p.captions} caption${p.captions === 1 ? "" : "s"}${p.language ? ` (${p.language})` : ""}.` : p.message || "No speech was found.",
        });
      else setResult({ ok: true, text: "Speech model ready." });
    });
    return () => unlisten(off);
  }, [refreshModels]);

  const captions = useMemo(
    () => project.items.filter((i) => i.kind === "caption").sort((a, b) => (a.start ?? 0) - (b.start ?? 0)),
    [project.items],
  );
  const selectedCaptions = selection.filter((i) => i.kind === "caption");
  const selectedClips = selection.filter((i) => (i.kind === "video" || i.kind === "audio") && i.mediaId);
  const sourceIds = useSelection ? selectedClips.map((i) => i.id) : [];
  const current = models.find((m) => m.id === model);
  const busy = job !== null;
  const hasClips = project.items.some((i) => (i.kind === "video" || i.kind === "audio") && i.mediaId);
  const styleTargets = styleScope === "all" || selectedCaptions.length === 0 ? [] : selectedCaptions.map((i) => i.id);

  const generate = async () => {
    setResult(null);
    const r = await commands.autoCaption({
      itemIds: sourceIds,
      language: language === "auto" ? null : language,
      model,
      presetId: style,
      replace,
    });
    if (r.status === "error") setResult({ ok: false, text: r.error.message });
    else setJob({ stage: "start", fraction: 0, message: "Starting…" });
  };

  const download = async () => {
    setResult(null);
    const r = await commands.downloadCaptionModel(model);
    if (r.status === "error") setResult({ ok: false, text: r.error.message });
    else setJob({ stage: "download", fraction: 0, message: "Starting the download…" });
  };

  const track = firstTrack(project, "caption", "t_captions");
  return (
    <div className="captions-panel">
      <section className="cap-auto" data-testid="caption-auto">
        <h5 className="panel-sub">Auto captions</h5>
        <p className="muted panel-note">Transcribed on this computer: your audio never leaves it.</p>
        <label className="select-row">
          Clips
          <select data-testid="caption-source" value={useSelection && selectedClips.length ? "selected" : "main"} onChange={(e) => setUseSelection(e.target.value === "selected")} disabled={busy}>
            <option value="main">Main track</option>
            <option value="selected" disabled={selectedClips.length === 0}>
              Selected clips{selectedClips.length ? ` (${selectedClips.length})` : ""}
            </option>
          </select>
        </label>
        <label className="select-row">
          Language
          <select data-testid="caption-language" value={language} onChange={(e) => setLanguage(e.target.value)} disabled={busy}>
            {CAPTION_LANGUAGES.map(([id, label]) => (
              <option key={id} value={id}>
                {label}
              </option>
            ))}
          </select>
        </label>
        <label className="select-row">
          Accuracy
          <select data-testid="caption-model" value={model} onChange={(e) => setModel(e.target.value)} disabled={busy}>
            {models.map((m) => (
              <option key={m.id} value={m.id}>
                {m.label} · {m.sizeMb} MB{m.installed ? " ✓" : ""}
              </option>
            ))}
          </select>
        </label>
        <label className="select-row">
          Style
          <select data-testid="caption-new-style" value={style} onChange={(e) => setStyle(e.target.value)} disabled={busy}>
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {p.label}
              </option>
            ))}
          </select>
        </label>
        <button className={`toggle-row${replace ? " on" : ""}`} onClick={() => setReplace(!replace)} disabled={busy} data-testid="caption-replace">
          Replace captions already there <span className="switch" />
        </button>
        {current && !current.installed && !busy && (
          <p className="muted panel-note cap-model-note" data-testid="caption-model-note">
            The {current.label.toLowerCase()} speech model ({current.sizeMb} MB) downloads the first time.{" "}
            <button className="link-btn" onClick={() => fire(download(), onError)} data-testid="caption-download">
              Download now
            </button>
          </p>
        )}
        {busy ? (
          <div className="cap-job" data-testid="caption-progress">
            <div className="cap-progress">
              <div className="cap-progress-bar" style={{ width: `${Math.round(job.fraction * 100)}%` }} />
            </div>
            <div className="cap-job-row">
              <span className="small">{job.message}</span>
              <button className="chip-sm" onClick={() => runCommand(commands.cancelCaptions(), undefined, onError)} data-testid="caption-cancel">
                Cancel
              </button>
            </div>
          </div>
        ) : (
          <button className="btn wide primary" data-testid="caption-generate" disabled={!hasClips} onClick={() => fire(generate(), onError)}>
            <Icon name="captions" size={15} /> Generate captions
          </button>
        )}
        {!hasClips && !busy && <p className="muted panel-note">Add a clip with sound to caption it.</p>}
        {result && (
          <p className={`panel-note cap-result ${result.ok ? "ok" : "err"}`} data-testid="caption-result">
            {result.text}
          </p>
        )}
      </section>

      <section>
        <div className="cap-head">
          <h5 className="panel-sub">Styles</h5>
          <div className="chips-row">
            <button className={`chip-sm${styleScope === "auto" && selectedCaptions.length ? " on" : ""}`} disabled={selectedCaptions.length === 0} onClick={() => setStyleScope("auto")}>
              Selected{selectedCaptions.length ? ` (${selectedCaptions.length})` : ""}
            </button>
            <button className={`chip-sm${styleScope === "all" || selectedCaptions.length === 0 ? " on" : ""}`} onClick={() => setStyleScope("all")} data-testid="caption-style-all">
              All captions
            </button>
          </div>
        </div>
        <div className="preset-grid cap-styles">
          {presets.map((p) => (
            <button
              key={p.id}
              className="preset-card"
              data-testid={`caption-style-${p.id}`}
              disabled={captions.length === 0}
              title={`${p.label}: ${animationLabel(p.animation)}, ${p.position}`}
              onClick={() => {
                setStyle(p.id);
                onEdit(commands.applyCaptionStyle(p.id, styleTargets));
              }}
            >
              <span className="preset-preview cap-preview" data-pos={p.position}>
                <CaptionSample preset={p} />
              </span>
              <span className="preset-label">
                {p.label}
                <em>{animationLabel(p.animation)}</em>
              </span>
            </button>
          ))}
        </div>
      </section>

      <section>
        <div className="cap-head">
          <h5 className="panel-sub">Transcript</h5>
          <button
            className="chip-sm"
            data-testid="caption-add"
            disabled={!track}
            onClick={() =>
              track &&
              onEdit(
                addItems(
                  [
                    {
                      trackId: track,
                      type: "caption",
                      startTime: snapToFrame(playhead),
                      endTime: snapToFrame(playhead + 3),
                      name: "Caption",
                      captionText: "Your words here",
                      captionSegments: [{ id: "s1", text: "Your words here", startTime: 0, endTime: 3 }],
                      captionPresetId: style,
                    },
                  ],
                  "add caption",
                ),
              )
            }
          >
            <Icon name="plus" size={13} /> Add at playhead
          </button>
        </div>
        {captions.length === 0 && <p className="muted panel-note">No captions yet. Generate them from the sound, or add one and type it.</p>}
        <div className="cap-transcript" data-testid="caption-transcript">
          {captions.map((c) => (
            <TranscriptRow key={c.id} item={c} selected={selection.some((s) => s.id === c.id)} onSelect={() => onSelect(c)} onEdit={onEdit} />
          ))}
        </div>
      </section>
    </div>
  );
}

function TranscriptRow({ item, selected, onSelect, onEdit }: { item: ItemView_Serialize; selected: boolean; onSelect: () => void; onEdit: PanelProps["onEdit"] }) {
  // The text being typed; null shows the caption's own text.
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? item.text ?? "";
  const commit = async (typed: string | null) => {
    setDraft(null);
    const next = (typed ?? "").replace(/\s+/g, " ").trim();
    if (typed === null || next === (item.text ?? "").trim() || !next) return;
    const d = await itemDetail(item.id);
    const segs = (Array.isArray(d?.captionSegments) ? d.captionSegments : []) as CaptionSegment[];
    // One segment per auto caption: the whole text goes into it, re-timed. Several typed lines: replace the text only.
    const segments = segs.length === 1 ? [retimeSegment(segs[0], next)] : segs.length === 0 ? [{ id: "s1", text: next, startTime: 0, endTime: (item.end ?? 0) - (item.start ?? 0) }] : null;
    onEdit(updateItem(item.id, segments ? { captionText: next, captionSegments: segments } : { captionText: next }, [], "edit caption text"));
  };
  return (
    <div className={`cap-line${selected ? " on" : ""}`} data-testid="caption-line">
      <button className="cap-time" onClick={onSelect} title="Open this caption in the inspector">
        {fmt(item.start ?? 0)}
      </button>
      <textarea
        rows={1}
        value={text}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={() => fire(commit(draft))}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            (e.target as HTMLTextAreaElement).blur();
          }
          if (e.key === "Escape") fire(commit(null));
          e.stopPropagation();
        }}
      />
    </div>
  );
}
