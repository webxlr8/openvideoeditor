// Spending by agents (unify-agent-tool-layer, D7): the price card an agent's generation request shows before anything
// is spent, and the per-session spending limit in Settings → Generation. Only the person's click on a card approves;
// the card's answer goes back to the one request that asked.

import { useEffect, useState } from "react";
import type { AppError, SpendRequested } from "../bindings";
import { commands, events } from "../bridge";
import { fire, logError, runCommand } from "../notify";
import "./spend.css";

/** Price cards from agents, one at a time, oldest first. Mounted once for the whole app. */
export function SpendCards() {
  const [cards, setCards] = useState<SpendRequested[]>([]);
  useEffect(() => {
    const shown = events.spendRequested.listen((e) =>
      setCards((cs) => [
        ...cs.filter((c) => c.quoteId !== e.payload.quoteId),
        e.payload,
      ]),
    );
    const settled = events.spendSettled.listen((e) =>
      setCards((cs) => cs.filter((c) => c.quoteId !== e.payload)),
    );
    return () => {
      fire(shown.then((off) => off()), logError);
      fire(settled.then((off) => off()), logError);
    };
  }, []);
  const card = cards[0];
  if (!card) return null;
  const answer = (approved: boolean) => {
    setCards((cs) => cs.filter((c) => c.quoteId !== card.quoteId));
    runCommand(commands.answerSpend(card.quoteId, approved), undefined, logError);
  };
  const usd = card.estimateUsd ?? 0;
  const price = `$${usd.toFixed(usd < 0.1 ? 3 : 2)}`;
  return (
    <div className="modal-backdrop" data-testid="spend-card-backdrop">
      <div
        className="modal spend-card"
        role="alertdialog"
        aria-modal="true"
        aria-label="Approve spending"
        data-testid="spend-card"
        data-quote={card.quoteId}
      >
        <header>
          <h3>Approve spending?</h3>
        </header>
        <p className="spend-price" data-testid="spend-price">
          {price}
        </p>
        <p className="spend-text" data-testid="spend-text">
          {card.text}
        </p>
        <p className="muted small">
          Nothing is spent unless you approve. Your provider bills you directly.
          {cards.length > 1 && ` ${cards.length - 1} more waiting.`}
        </p>
        <div className="xp-actions">
          <button
            className="btn"
            autoFocus
            data-testid="spend-decline"
            onClick={() => answer(false)}
          >
            Decline
          </button>
          <button
            className="btn primary"
            data-testid="spend-approve"
            onClick={() => answer(true)}
          >
            Approve {price}
          </button>
        </div>
      </div>
    </div>
  );
}

/** Settings → Generation: the most one agent session may spend on generation. */
export function SpendCapField({ onError }: { onError: (e: AppError) => void }) {
  const [value, setValue] = useState("");
  const [problem, setProblem] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    fire(
      commands.spendCap().then((v) => setValue((v ?? 5).toFixed(2))),
      logError,
    );
  }, []);
  const save = () => {
    const n = Number(value.trim().replace(/^\$/, ""));
    if (!value.trim() || !Number.isFinite(n) || n < 0)
      return setProblem("The limit is an amount in dollars, $0 or more.");
    setProblem(null);
    runCommand(
      commands.setSpendCap(n),
      (v) => {
        setValue((v ?? n).toFixed(2));
        setSaved(true);
      },
      (e) => {
        setProblem(e.message);
        onError(e);
      },
    );
  };
  return (
    <div className="spend-cap" data-testid="spend-cap">
      <label htmlFor="spend-cap-input">
        Spending limit per agent session ($)
        <br />
        <span className="muted small">
          Agents (the assistant, or one connected over MCP) ask you to approve
          each request&apos;s price. One session never spends more than this in
          total; $0 stops agents from generating.
        </span>
      </label>
      <div className="gen-key-row">
        <input
          id="spend-cap-input"
          inputMode="decimal"
          value={value}
          data-testid="spend-cap-input"
          onChange={(e) => {
            setValue(e.target.value);
            setSaved(false);
          }}
        />
        <button className="btn" onClick={save} data-testid="spend-cap-save">
          {saved ? "Saved" : "Save"}
        </button>
      </div>
      {problem && <p className="key-error">{problem}</p>}
    </div>
  );
}
