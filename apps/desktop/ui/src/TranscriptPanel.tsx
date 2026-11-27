// Transcript panel (add-transcript-editing): the words said on the timeline, following playback. Click a word to
// seek; select words (click, Shift-click for a run) to strike them through, then press Delete to preview the cut and
// apply it as one undo step. "Remove pauses" and "Remove fillers" open review lists first; the search box finds a
// phrase in every transcribed file. Each action runs the same tool agents use (`get_transcript`, `remove_words`, …).

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { commands } from "./bridge";
import { setCutPreview } from "./cutPreview";
import type { PanelProps } from "./Panels";
import { PanelHint } from "./onboarding/PanelHint";
import "./TranscriptPanel.css";

export type Word = {
  id: string;
  text: string;
  start: number;
  end: number;
  clipId: string;
  filler: boolean;
  contextFiller?: boolean;
};

type Filler = {
  id?: string;
  kind: string;
  text?: string;
  start: number;
  end: number;
  selected: boolean;
  context: boolean;
};
type Hit = {
  mediaId: string;
  text: string;
  sourceStart: number;
  wordIds: string[];
  uses: { clipId: string; start: number }[];
};
type Range = { start: number; end: number };

/** A cut waiting for the person's confirmation: the tool, its arguments and the dry run's result. */
type Preview = {
  tool: "remove_words" | "remove_silence";
  args: Record<string, unknown>;
  summary: string;
  diff: string;
  ranges: Range[];
};

/** Sentences: a new one after `.`, `?` or `!`, at a clip change, or after a gap of more than a second. */
export function sentences(words: Word[]): Word[][] {
  const out: Word[][] = [];
  let open = false;
  for (const w of words) {
    const last = out[out.length - 1];
    const prev = last?.[last.length - 1];
    if (!last || !open || prev.clipId !== w.clipId || w.start - prev.end > 1)
      out.push([w]);
    else last.push(w);
    open = !/[.?!]\s*$/.test(w.text);
  }
  return out;
}

const time = (t: number) => {
  const m = Math.floor(t / 60);
  return `${m}:${(t - m * 60).toFixed(1).padStart(4, "0")}`;
};

async function query<T>(
  tool: string,
  args: Record<string, unknown>,
): Promise<T> {
  const r = await commands.transcriptQuery(tool, JSON.stringify(args));
  if (r.status === "error")
    throw Object.assign(new Error(r.error.message), r.error);
  return JSON.parse(r.data) as T;
}

export function TranscriptPanel({
  project,
  playhead,
  onEdit,
  onError,
  onSeek,
  onImport,
}: PanelProps) {
  const [words, setWords] = useState<Word[] | null>(null);
  const [untranscribed, setUntranscribed] = useState<string[]>([]);
  const [selected, setSelected] = useState<string[]>([]);
  const [anchor, setAnchor] = useState<string | null>(null);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [fillers, setFillers] = useState<
    (Filler & { checked: boolean })[] | null
  >(null);
  const [phrase, setPhrase] = useState("");
  const [hits, setHits] = useState<Hit[] | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const listRef = useRef<HTMLDivElement>(null);

  // The previewed cut is drawn over the clips on the timeline until it is applied or cancelled.
  useEffect(() => {
    setCutPreview(preview?.ranges ?? []);
  }, [preview]);
  useEffect(() => () => setCutPreview([]), []);

  const fail = useCallback(
    (e: unknown) => {
      const err = e as { code?: string; message?: string };
      setMessage(err.message ?? String(e));
      if (err.code && onError) onError(err as never);
    },
    [onError],
  );

  const refresh = useCallback(() => {
    query<{ words: Word[]; untranscribed: string[] }>("get_transcript", {})
      .then((t) => {
        setWords(t.words);
        setUntranscribed(t.untranscribed);
      })
      .catch(fail);
  }, [fail]);

  // Read again after every revision (a cut here, an edit elsewhere).
  useEffect(refresh, [refresh, project.rev]);

  const current = useMemo(
    () =>
      words?.find((w) => playhead >= w.start && playhead < w.end)?.id ?? null,
    [words, playhead],
  );
  useEffect(() => {
    if (!current) return;
    listRef.current
      ?.querySelector(`[data-word="${CSS.escape(current)}"]`)
      ?.scrollIntoView?.({ block: "nearest" });
  }, [current]);

  const pick = (w: Word, e: React.MouseEvent) => {
    if (e.shiftKey && anchor && words) {
      const a = words.findIndex((x) => x.id === anchor);
      const b = words.findIndex((x) => x.id === w.id);
      const run = words
        .slice(Math.min(a, b), Math.max(a, b) + 1)
        .map((x) => x.id);
      setSelected((s) => Array.from(new Set([...s, ...run])));
    } else if (e.ctrlKey || e.metaKey) {
      setSelected((s) =>
        s.includes(w.id) ? s.filter((x) => x !== w.id) : [...s, w.id],
      );
      setAnchor(w.id);
    } else {
      setSelected([w.id]);
      setAnchor(w.id);
      onSeek?.(w.start);
    }
  };

  const dryRun = async (
    tool: Preview["tool"],
    args: Record<string, unknown>,
  ) => {
    setBusy(true);
    setMessage(null);
    try {
      const r = await query<{
        summary: string;
        diff: string;
        ranges: Range[];
        speechFound?: boolean;
        message?: string;
      }>(tool, args);
      if (!r.ranges?.length) setMessage(r.message ?? "Nothing to cut.");
      else
        setPreview({
          tool,
          args,
          summary: r.summary,
          diff: r.diff,
          ranges: r.ranges,
        });
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  };

  const cutSelected = () => {
    if (selected.length) dryRun("remove_words", { ids: selected }).catch(fail);
  };

  const apply = async () => {
    if (!preview) return;
    setBusy(true);
    const p = commands.transcriptCut(
      preview.tool,
      JSON.stringify(preview.args),
    );
    onEdit(p);
    const r = await p;
    setBusy(false);
    if (r.status === "error") {
      fail(r.error);
      return;
    }
    setMessage(`Done: ${preview.summary}. Undo puts it back.`);
    setPreview(null);
    setSelected([]);
    setFillers(null);
  };

  const reviewFillers = () => {
    setBusy(true);
    query<{ fillers: Filler[] }>("find_fillers", {})
      .then((r) =>
        setFillers(r.fillers.map((f) => ({ ...f, checked: f.selected }))),
      )
      .catch(fail)
      .finally(() => setBusy(false));
  };

  const search = (e: React.FormEvent) => {
    e.preventDefault();
    if (!phrase.trim()) return setHits(null);
    query<{ hits: Hit[] }>("search_transcript", { phrase })
      .then((r) => setHits(r.hits))
      .catch(fail);
  };

  const onKey = (e: React.KeyboardEvent) => {
    if (
      (e.key === "Delete" || e.key === "Backspace") &&
      selected.length &&
      !preview
    ) {
      e.preventDefault();
      cutSelected();
    } else if (e.key === "Escape") {
      setSelected([]);
      setPreview(null);
    }
  };

  const hitWords = new Set(hits?.flatMap((h) => h.wordIds) ?? []);

  return (
    <div
      className="transcript-panel"
      data-testid="transcript-panel"
      onKeyDown={onKey}
      tabIndex={-1}
    >
      {project.items.length === 0 && (
        <PanelHint panel="transcript" action="Import media" onAction={onImport}>
          Edit by text: cut words, pauses and fillers from what was said.
        </PanelHint>
      )}
      <form className="transcript-search" onSubmit={search}>
        <input
          className="input"
          type="search"
          placeholder="Search what was said"
          value={phrase}
          onChange={(e) => setPhrase(e.target.value)}
          data-testid="transcript-search"
          aria-label="Search the transcript"
        />
      </form>
      {hits && (
        <ul className="transcript-hits" data-testid="transcript-hits">
          {hits.length === 0 && <li className="muted small">No match.</li>}
          {hits.map((h) => (
            <li key={h.wordIds[0]}>
              <button
                className="link"
                onClick={() => h.uses[0] && onSeek?.(h.uses[0].start)}
                disabled={!h.uses.length}
                data-testid={`transcript-hit-${h.wordIds[0]}`}
              >
                “{h.text}”{" "}
                {h.uses.length
                  ? `at ${time(h.uses[0].start)}`
                  : "(not on the timeline)"}
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="transcript-actions">
        <button
          className="btn"
          disabled={!selected.length || busy}
          onClick={cutSelected}
          data-testid="transcript-cut"
        >
          Cut{" "}
          {selected.length
            ? `${selected.length} word${selected.length === 1 ? "" : "s"}`
            : "words"}
        </button>
        <button
          className="btn"
          disabled={busy}
          onClick={() => {
            dryRun("remove_silence", {}).catch(fail);
          }}
          data-testid="transcript-pauses"
        >
          Remove pauses
        </button>
        <button
          className="btn"
          disabled={busy}
          onClick={reviewFillers}
          data-testid="transcript-fillers"
        >
          Remove fillers
        </button>
      </div>
      {message && (
        <p className="small" data-testid="transcript-message">
          {message}
        </p>
      )}
      {preview && (
        <div
          className="transcript-preview"
          data-testid="transcript-preview"
          role="dialog"
          aria-label="Preview the cut"
        >
          <p className="small">
            <strong>{preview.summary}</strong> · {preview.ranges.length} cut
            {preview.ranges.length === 1 ? "" : "s"}
          </p>
          <ul
            className="transcript-ranges small"
            data-testid="transcript-ranges"
          >
            {preview.ranges.map((r) => (
              <li key={`${r.start}-${r.end}`}>
                <button className="link" onClick={() => onSeek?.(r.start)}>
                  {time(r.start)}–{time(r.end)}
                </button>
              </li>
            ))}
          </ul>
          {preview.diff && (
            <details>
              <summary className="small">timeline.tsx change</summary>
              <pre className="transcript-diff" data-testid="transcript-diff">
                {preview.diff}
              </pre>
            </details>
          )}
          <div className="transcript-actions">
            <button
              className="btn primary"
              onClick={() => {
                apply().catch(fail);
              }}
              disabled={busy}
              data-testid="transcript-apply"
            >
              Cut
            </button>
            <button
              className="btn"
              onClick={() => setPreview(null)}
              data-testid="transcript-cancel"
            >
              Cancel
            </button>
          </div>
        </div>
      )}
      {fillers && !preview && (
        <div className="transcript-review" data-testid="transcript-filler-list">
          {fillers.length === 0 && (
            <p className="muted small">No fillers found.</p>
          )}
          <ul>
            {fillers.map((f, i) => (
              <li key={`${f.id ?? f.kind}-${f.start}`}>
                <label className="small">
                  <input
                    type="checkbox"
                    checked={f.checked}
                    disabled={!f.id}
                    onChange={() =>
                      setFillers(
                        (x) =>
                          x &&
                          x.map((y, j) =>
                            j === i ? { ...y, checked: !y.checked } : y,
                          ),
                      )
                    }
                    data-testid={`transcript-filler-${f.id ?? i}`}
                  />{" "}
                  {f.text || "(voiced sound)"}{" "}
                  {f.context && (
                    <span className="muted">· depends on context</span>
                  )}
                </label>{" "}
                <button
                  className="link small"
                  onClick={() => onSeek?.(f.start)}
                  aria-label="Play from here"
                >
                  {time(f.start)}
                </button>
              </li>
            ))}
          </ul>
          <div className="transcript-actions">
            <button
              className="btn primary"
              disabled={!fillers.some((f) => f.checked && f.id) || busy}
              onClick={() => {
                dryRun("remove_words", {
                  ids: fillers
                    .filter((f) => f.checked && f.id)
                    .map((f) => f.id),
                }).catch(fail);
              }}
              data-testid="transcript-fillers-cut"
            >
              Cut {fillers.filter((f) => f.checked && f.id).length} selected
            </button>
            <button className="btn" onClick={() => setFillers(null)}>
              Close
            </button>
          </div>
        </div>
      )}
      {words === null && <p className="muted small">Reading the transcript…</p>}
      {words && words.length === 0 && (
        <p className="muted small panel-note" data-testid="transcript-empty">
          {untranscribed.length
            ? "No words yet: transcribe the clips first (Captions panel, or “ove transcript transcribe”)."
            : "There is no clip with speech on the timeline."}
        </p>
      )}
      <div
        className="transcript-words"
        ref={listRef}
        data-testid="transcript-words"
      >
        {words &&
          sentences(words).map((s) => (
            <p key={s[0].id} className="transcript-row">
              <span className="transcript-time muted small">
                {time(s[0].start)}
              </span>{" "}
              {s.map((w) => (
                <span
                  key={w.id}
                  data-word={w.id}
                  data-testid={`word-${w.id}`}
                  className={[
                    "transcript-word",
                    selected.includes(w.id) ? "struck" : "",
                    w.id === current ? "current" : "",
                    hitWords.has(w.id) ? "hit" : "",
                    w.filler ? "filler" : "",
                  ].join(" ")}
                  onClick={(e) => pick(w, e)}
                >
                  {w.text}{" "}
                </span>
              ))}
            </p>
          ))}
      </div>
    </div>
  );
}
