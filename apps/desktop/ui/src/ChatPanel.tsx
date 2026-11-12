import { Fragment, useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import type {
  AgentStatus_Serialize,
  AppError,
  ChatItem_Serialize,
  ChatSummaryView,
  KeyCheck,
  TurnReview,
  TurnUsageView,
  UsageTotals_Serialize,
} from "./bindings";
import { commands, events } from "./bridge";
import { Icon } from "./icons";
import { Dropdown } from "./Menu";
import { fire, reportError, runCommand, unlisten } from "./notify";

interface Item {
  role: "user" | "assistant" | "tool" | "question" | "note" | "error" | "changes" | "usage";
  text: string;
  id?: string | null;
  ok?: boolean | null;
  summary?: string;
  options?: string[];
  answered?: boolean;
  review?: TurnReview | null;
  usage?: TurnUsageView | null;
}

function fromChat(items: ChatItem_Serialize[]): Item[] {
  return items.map((i) => ({
    role: i.role as Item["role"],
    text: i.text,
    id: i.id,
    ok: i.ok,
    options: i.options,
    // A reloaded log belongs to a finished turn, so its questions are closed.
    answered: i.role === "question" ? true : undefined,
    review: i.review,
    usage: i.usage,
  }));
}

/** Dollars: four decimals under a cent, two above. */
export function usd(v: number | null | undefined): string {
  const c = v ?? 0;
  return `$${c < 0.01 ? c.toFixed(4) : c.toFixed(2)}`;
}

/** Token counts: 950, 12.3k, 1.2M. */
export function tokens(v: number | null | undefined): string {
  const n = v ?? 0;
  if (n < 1000) return String(Math.round(n));
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

/** Inline formatting: **bold** and `code`. */
function inline(text: string): ReactNode[] {
  const parts = text.split(/(\*\*[^*]+\*\*|`[^`]+`)/g);
  return parts.map((p, i) =>
    p.startsWith("**") && p.endsWith("**") ? (
      <strong key={i}>{p.slice(2, -2)}</strong>
    ) : p.startsWith("`") && p.endsWith("`") ? (
      <code key={i}>{p.slice(1, -1)}</code>
    ) : (
      <Fragment key={i}>{p}</Fragment>
    ),
  );
}

/** Markdown-lite: paragraphs, bullet and numbered lists, bold, inline code. */
export function MarkdownLite({ text }: { text: string }) {
  const blocks: ReactNode[] = [];
  let list: { ordered: boolean; items: string[] } | null = null;
  const flush = () => {
    if (!list) return;
    const items = list.items.map((t, i) => <li key={i}>{inline(t)}</li>);
    blocks.push(list.ordered ? <ol key={blocks.length}>{items}</ol> : <ul key={blocks.length}>{items}</ul>);
    list = null;
  };
  for (const line of text.split("\n")) {
    const bullet = line.match(/^\s*[-*]\s+(.*)$/);
    const num = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (bullet || num) {
      const ordered = !!num;
      if (!list || list.ordered !== ordered) {
        flush();
        list = { ordered, items: [] };
      }
      list.items.push((bullet ?? num)![1]);
      continue;
    }
    flush();
    if (line.trim()) blocks.push(<p key={blocks.length}>{inline(line)}</p>);
  }
  flush();
  return <>{blocks}</>;
}

let pendingDraft: string | null = null;

/** Prefills the next chat composer that mounts (used when a Home prompt could not be sent). */
export function setPendingDraft(text: string) {
  pendingDraft = text;
}

function takeDraft(): string {
  const d = pendingDraft ?? "";
  pendingDraft = null;
  return d;
}

function shortModel(label: string) {
  return label.replace(/^Claude /, "");
}

export function ChatPanel({
  onClose,
  projectId,
  draft,
  onError,
  onSeek,
  shownTurn,
  onShowTurn,
}: {
  onClose: () => void;
  /** Changes when another project opens (the chat list is per project). */
  projectId?: string | null;
  /** Text to prefill the composer with (e.g. a prompt typed on Home before a key was set). */
  draft?: string;
  onError?: (e: AppError) => void;
  /** The assistant's `seek_viewer` tool: show the timeline at this time (seconds). */
  onSeek?: (seconds: number) => void;
  /** The turn whose changes the timeline highlights, if any. */
  shownTurn?: string | null;
  /** Highlights a turn's changed items on the timeline (`null`: stop). */
  onShowTurn?: (turnId: string | null, ids: string[]) => void;
}) {
  const [status, setStatus] = useState<AgentStatus_Serialize | null>(null);
  const [chats, setChats] = useState<ChatSummaryView[]>([]);
  const [chatId, setChatId] = useState<string | null>(null);
  const [items, setItems] = useState<Item[]>([]);
  const [running, setRunning] = useState(false);
  const [thinking, setThinking] = useState(false);
  const [text, setText] = useState(() => draft ?? takeDraft());
  const [footer, setFooter] = useState<string | null>(null);
  const [totals, setTotals] = useState<UsageTotals_Serialize | null>(null);
  const [keyCheck, setKeyCheck] = useState<KeyCheck | null>(null);
  const chatRef = useRef<string | null>(null);
  const scroller = useRef<HTMLDivElement>(null);

  const report = useCallback((e: AppError) => (onError ? onError(e) : reportError(e)), [onError]);

  const loadChats = useCallback(
    () => runCommand(commands.agentChats(), setChats, report),
    [report],
  );

  const loadChat = useCallback(
    (id: string) => runCommand(commands.agentChat(id), (c) => setItems(fromChat(c)), report),
    [report],
  );

  const loadTotals = useCallback(
    (id: string | null) => runCommand(commands.agentUsage(id), setTotals, report),
    [report],
  );

  useEffect(() => loadTotals(chatId), [chatId, projectId, loadTotals]);

  useEffect(() => {
    runCommand(
      commands.agentStatus(),
      (s) => {
        setStatus(s);
        setRunning(s.running);
      },
      report,
    );
  }, [report]);

  const shownProject = useRef(projectId);
  useEffect(() => {
    if (shownProject.current !== projectId) {
      // Chats live in the project: another project starts with a fresh view.
      shownProject.current = projectId;
      chatRef.current = null;
      queueMicrotask(() => {
        setChatId(null);
        setItems([]);
      });
    }
    loadChats();
  }, [projectId, loadChats]);

  useEffect(() => {
    const un = events.agentUiEvent.listen((e) => {
      const ev = e.payload;
      switch (ev.kind) {
        case "turnStarted":
          if (chatRef.current !== ev.chatId) {
            // A turn this panel did not start (e.g. the Home prompt): show what led to it.
            runCommand(
              commands.agentChat(ev.chatId),
              (chat) => setItems((cur) => [...fromChat(chat), ...cur.filter((x) => x.role !== "user")]),
              report,
            );
          }
          chatRef.current = ev.chatId;
          setChatId(ev.chatId);
          setRunning(true);
          setFooter(null);
          break;
        case "textDelta":
          setThinking(false);
          setItems((xs) => {
            const last = xs[xs.length - 1];
            if (last && last.role === "assistant" && last.id === "live") {
              return [...xs.slice(0, -1), { ...last, text: last.text + ev.text }];
            }
            return [...xs, { role: "assistant", text: ev.text, id: "live" }];
          });
          break;
        case "thinkingDelta":
          setThinking(true);
          break;
        case "toolCall":
          setThinking(false);
          setItems((xs) => [...xs.map((x) => (x.id === "live" ? { ...x, id: null } : x)), { role: "tool", text: ev.label, id: ev.id, ok: null }]);
          break;
        case "toolResult":
          setItems((xs) => xs.map((x) => (x.id === ev.id && x.role === "tool" ? { ...x, ok: ev.ok, summary: ev.summary } : x)));
          break;
        case "question":
          setItems((xs) => [...xs, { role: "question", text: ev.question, id: ev.id, options: ev.options, answered: false }]);
          break;
        case "retrying":
          setFooter(`${ev.message} (in ${Math.round(ev.seconds ?? 0)}s)`);
          break;
        case "error":
          setItems((xs) => [...xs, { role: "error", text: ev.message }]);
          break;
        case "turnDone": {
          setRunning(false);
          setThinking(false);
          const cost = ev.costUsd ?? 0;
          const secs = (ev.durationMs ?? 0) / 1000;
          // A limit the turn stopped at shows in the log (it is kept there), not twice.
          setFooter(ev.stopped ? "Stopped." : ev.note ? null : `${secs.toFixed(1)}s · ≈${usd(cost)}`);
          loadChat(ev.chatId);
          loadChats();
          loadTotals(ev.chatId);
          break;
        }
        case "seek":
          if (ev.seconds != null && Number.isFinite(ev.seconds)) onSeek?.(Math.max(0, ev.seconds));
          break;
      }
    });
    return () => unlisten(un);
  }, [loadChat, loadChats, loadTotals, report, onSeek]);

  useEffect(() => {
    scroller.current?.scrollTo?.({ top: scroller.current.scrollHeight });
  }, [items, thinking]);

  const send = async () => {
    const t = text.trim();
    if (!t || running) return;
    setText("");
    setItems((xs) => [...xs, { role: "user", text: t }]);
    setRunning(true);
    const r = await commands.agentSend(chatRef.current, t);
    if (r.status === "error") {
      setRunning(false);
      setItems((xs) => [...xs, { role: "error", text: r.error.message }]);
      setText(t);
      return;
    }
    chatRef.current = r.data.chatId;
    setChatId(r.data.chatId);
  };

  const answer = async (id: string, value: string) => {
    setItems((xs) => xs.map((x) => (x.id === id && x.role === "question" ? { ...x, answered: true, text: `${x.text}\n→ ${value}` } : x)));
    const r = await commands.agentAnswer(id, value);
    if (r.status === "error") report(r.error);
  };

  const openChat = (id: string) => {
    chatRef.current = id;
    setChatId(id);
    onShowTurn?.(null, []);
    loadChat(id);
  };

  const newChat = () => {
    chatRef.current = null;
    setChatId(null);
    setItems([]);
    setFooter(null);
    onShowTurn?.(null, []);
  };

  const undoTurn = (turnId: string) => {
    const id = chatRef.current;
    if (!id) return;
    if (shownTurn === turnId) onShowTurn?.(null, []);
    runCommand(commands.agentUndoTurn(id, turnId), () => loadChat(id), report);
  };

  const model = status?.models.find((m) => m.id === status.model);

  return (
    <aside className="chat" data-testid="chat">
      <header className="chat-header">
        <button className="pill" onClick={newChat} data-testid="new-chat" title="Start a new conversation">
          New conversation
        </button>
        <span className="chat-header-tools">
          <Dropdown
            testId="chat-history"
            label=""
            icon={<Icon name="history" size={16} />}
            items={
              chats.length
                ? chats.map((c) => ({ id: c.id, label: c.title, hint: c.id === chatId ? "open" : undefined, onSelect: () => openChat(c.id) }))
                : [{ id: "none", label: "No conversations yet" }]
            }
          />
          <button className="icon-btn" title="Hide assistant" onClick={onClose} data-testid="close-chat">
            <Icon name="close" size={16} />
          </button>
        </span>
      </header>

      {totals && (totals.project.turns ?? 0) > 0 && (
        <div
          className="chat-usage muted small"
          data-testid="chat-usage"
          title={
            `This project, all chats: ${tokens(totals.project.inputTokens)} tokens in, ${tokens(totals.project.outputTokens)} out (${tokens(totals.project.cacheReadTokens)} read from cache), ≈${usd(totals.project.costUsd)}.\n` +
            `This month: ≈${usd(totals.month.costUsd)}${totals.monthCapUsd != null ? ` of your ${usd(totals.monthCapUsd)} cap` : ""}. Estimates from the API's token counts; Anthropic bills you directly.`
          }
        >
          {totals.chat && (totals.chat.turns ?? 0) > 0 && (
            <span>
              This chat: {tokens(totals.chat.inputTokens)} in · {tokens(totals.chat.outputTokens)} out · ≈{usd(totals.chat.costUsd)}
            </span>
          )}
          <span>Project: ≈{usd(totals.project.costUsd)}</span>
        </div>
      )}
      {status && !status.hasKey ? (
        <KeyCard
          onSaved={(s, check) => {
            setStatus(s);
            setKeyCheck(check);
          }}
        />
      ) : items.length === 0 ? (
        <div className="chat-empty">
          <Icon name="spark" size={34} />
          {keyCheck && (
            <p className={keyCheck.ok ? "key-ok" : "key-error"} role="status" data-testid="key-check">
              {keyCheck.ok ? "Key works. " : ""}
              {keyCheck.ok ? "You're ready." : keyCheck.message}
            </p>
          )}
          <p>Ask for a cut, captions, or a fresh idea.</p>
          <p className="muted small">I edit this project's timeline; every change is one undo step.</p>
        </div>
      ) : (
        <div className="chat-log" ref={scroller} data-testid="chat-log">
          {items.map((it, i) => (
            <ChatLine
              key={i}
              item={it}
              onAnswer={(id, value) => fire(answer(id, value), report)}
              shown={!!it.review && it.review.turnId === shownTurn}
              onShow={(r) => onShowTurn?.(r ? r.turnId : null, r ? r.changedIds : [])}
              onUndoTurn={undoTurn}
              busy={running}
            />
          ))}
          {running && thinking && <div className="chat-thinking muted small">Thinking…</div>}
        </div>
      )}

      {footer && <div className="chat-footer muted small" data-testid="chat-footer">{footer}</div>}
      <div className="composer">
        <textarea
          placeholder={status && !status.hasKey ? "Add your API key above to start." : "Describe an edit…"}
          rows={3}
          value={text}
          data-testid="composer"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              fire(send(), report);
            }
          }}
        />
        <div className="composer-row">
          <button className="icon-btn" title="Attach (later)" disabled>
            <Icon name="attach" size={16} />
          </button>
          <span className="grow" />
          {status && (
            <Dropdown
              testId="agent-picker"
              direction="up"
              label={model ? shortModel(model.label) : "Model"}
              items={[
                ...status.models.map((m) => ({
                  id: m.id,
                  label: m.label,
                  hint: m.id === status.model ? "✓" : undefined,
                  onSelect: () => runCommand(commands.agentSetModel(m.id), setStatus, report),
                })),
                ...(status.hasKey && status.keySource === "keychain"
                  ? [
                      {
                        id: "forget",
                        label: "Remove API key",
                        danger: true,
                        onSelect: () => runCommand(commands.agentClearKey(), setStatus, report),
                      },
                    ]
                  : []),
              ]}
            />
          )}
          {running ? (
            <button className="send stop" title="Stop" data-testid="stop" onClick={() => runCommand(commands.agentStop(), undefined, report)}>
              <span className="stop-square" />
            </button>
          ) : (
            <button
              className="send"
              title="Send (Enter)"
              data-testid="send"
              disabled={!text.trim() || !status?.hasKey}
              onClick={() => fire(send(), report)}
            >
              <Icon name="send" size={16} />
            </button>
          )}
        </div>
      </div>
    </aside>
  );
}

function ChatLine({
  item,
  onAnswer,
  shown = false,
  onShow,
  onUndoTurn,
  busy = false,
}: {
  item: Item;
  onAnswer: (id: string, value: string) => void;
  /** This turn's changes are highlighted on the timeline. */
  shown?: boolean;
  onShow?: (review: TurnReview | null) => void;
  onUndoTurn?: (turnId: string) => void;
  /** A turn is running (undoing one waits for it). */
  busy?: boolean;
}) {
  const [custom, setCustom] = useState("");
  switch (item.role) {
    case "changes": {
      const r = item.review;
      if (!r) return null;
      const n = r.count;
      return (
        <div className={`turn-review${r.undone ? " undone" : ""}`} data-testid="turn-review">
          <div className="turn-review-row">
            <span className="grow">{n > 0 ? `Changed ${n} item${n === 1 ? "" : "s"}` : "Changed the project"}</span>
            {r.undone ? (
              <span className="muted small">Undone</span>
            ) : (
              <>
                {(r.changedIds.length > 0 || r.removed.length > 0) && (
                  <button
                    className={`btn small${shown ? " on" : ""}`}
                    aria-pressed={shown}
                    onClick={() => onShow?.(shown ? null : r)}
                    data-testid="turn-show"
                  >
                    {shown ? "Hide" : "Show"}
                  </button>
                )}
                <button
                  className="btn small"
                  disabled={busy}
                  title="Revert everything this turn changed, as one step you can undo"
                  onClick={() => onUndoTurn?.(r.turnId)}
                  data-testid="turn-undo"
                >
                  Undo this turn
                </button>
              </>
            )}
          </div>
          {shown && r.removed.length > 0 && (
            <div className="muted small" data-testid="turn-removed">
              Removed: {r.removed.join(", ")}
            </div>
          )}
          {shown && r.changedIds.length > 0 && (
            <div className="muted small">Highlighted on the timeline.</div>
          )}
        </div>
      );
    }
    case "usage": {
      const u = item.usage;
      if (!u) return null;
      return (
        <div className="turn-usage muted small" data-testid="turn-usage">
          {((u.durationMs ?? 0) / 1000).toFixed(1)}s · {tokens(u.inputTokens)} in · {tokens(u.outputTokens)} out · ≈
          {usd(u.costUsd)}
        </div>
      );
    }
    case "user":
      return <div className="msg msg-user" data-testid="msg-user">{item.text}</div>;
    case "assistant":
      return (
        <div className="msg msg-assistant" data-testid="msg-assistant">
          <MarkdownLite text={item.text} />
        </div>
      );
    case "tool":
      return (
        <div className={`tool-chip${item.ok === false ? " failed" : ""}`} title={item.summary} data-testid="tool-chip">
          <span className={`dot${item.ok === null || item.ok === undefined ? " busy" : ""}`} />
          {item.text}
          {item.summary && item.ok !== null && <span className="muted"> · {item.summary}</span>}
        </div>
      );
    case "question":
      return (
        <div className="question-card" data-testid="question-card">
          <div className="question-text">{item.text}</div>
          {!item.answered && item.id && (
            <>
              <div className="question-options">
                {(item.options ?? []).map((o) => (
                  <button key={o} className="chip" onClick={() => onAnswer(item.id!, o)}>
                    {o}
                  </button>
                ))}
              </div>
              <input
                className="question-input"
                placeholder="Or type an answer…"
                value={custom}
                onChange={(e) => setCustom(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && custom.trim()) onAnswer(item.id!, custom.trim());
                }}
              />
            </>
          )}
        </div>
      );
    case "note":
      return <div className="msg-note muted small">{item.text}</div>;
    case "error":
      return (
        <div className="msg-error" role="alert" data-testid="msg-error">
          {item.text}
        </div>
      );
  }
}

/** What the assistant sends to Anthropic, and what it never sends. */
export function AssistantDataList() {
  return (
    <ul className="data-list small" data-testid="assistant-data">
      <li>
        Sent to Anthropic: the timeline as text (item names and times, text and caption words, media file names), this
        chat, and the results of the assistant's tools.
      </li>
      <li>Never sent: your media files or video frames.</li>
      <li>Nothing else leaves this computer, and there is no telemetry.</li>
    </ul>
  );
}

function KeyCard({ onSaved }: { onSaved: (s: AgentStatus_Serialize, check: KeyCheck | null) => void }) {
  const [key, setKey] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [unchecked, setUnchecked] = useState<AgentStatus_Serialize | null>(null);
  const save = async () => {
    const r = await commands.agentSetKey(key);
    if (r.status === "error") return setError(r.error.message);
    setKey("");
    // Try the key for real (a free token count) so a wrong key shows up now, not at the first message.
    setChecking(true);
    const c = await commands.agentCheckKey().finally(() => setChecking(false));
    const check = c.status === "ok" ? c.data : null;
    if (check && !check.ok) {
      setError(check.message);
      // Keep the card up with the reason. The key stays saved: a network problem is not the key's fault.
      setUnchecked(r.data);
      return;
    }
    onSaved(r.data, check);
  };
  return (
    <div className="key-card" data-testid="key-card">
      <Icon name="spark" size={26} />
      <h3>Connect your assistant</h3>
      <p className="muted">
        The assistant runs on your own Anthropic API key, kept in your system's credential store. You can create a key
        in the Anthropic Console.
      </p>
      <AssistantDataList />
      <input
        type="password"
        placeholder="sk-ant-…"
        value={key}
        aria-label="Anthropic API key"
        data-testid="key-input"
        onChange={(e) => {
          setKey(e.target.value);
          setError(null);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") fire(save(), (err) => setError(err.message));
        }}
      />
      {error && <p className="key-error" data-testid="key-error">{error}</p>}
      <button
        className="btn primary"
        disabled={!key.trim() || checking}
        onClick={() => fire(save(), (err) => setError(err.message))}
        data-testid="key-save"
      >
        {checking ? "Checking the key…" : "Save key"}
      </button>
      {unchecked && (
        <button className="btn" onClick={() => onSaved(unchecked, null)} data-testid="key-continue">
          Use the saved key anyway
        </button>
      )}
    </div>
  );
}
